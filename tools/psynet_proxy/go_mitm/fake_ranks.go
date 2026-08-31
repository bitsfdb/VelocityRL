package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"log"
	"strconv"
	"strings"
)

type FakeRewardLevels struct {
	SeasonLevel     *int `json:"season_level"`
	SeasonLevelWins *int `json:"season_level_wins"`
}

func (r *FakeRewardLevels) hasAny() bool {
	if r == nil {
		return false
	}
	return r.SeasonLevel != nil || r.SeasonLevelWins != nil
}

type FakeRanksConfig struct {
	Enabled bool `json:"enabled"`

	Default *FakeRankOverride `json:"default"`

	Playlists map[string]*FakeRankOverride `json:"playlists"`

	RewardLevels *FakeRewardLevels `json:"reward_levels"`
}

type FakeRankOverride struct {
	DisplayMMR *float64 `json:"display_mmr"`
	Mu         *float64 `json:"mu"`
	Sigma      *float64 `json:"sigma"`
	Tier       *int     `json:"tier"`
	Division   *int     `json:"division"`
	WinStreak  *int     `json:"win_streak"`
}

const (
	fakeRankDisplayScale  = 20.0
	fakeRankDisplayOffset = 100.0
)

func (c SpoofConfig) fakeRanksActive() bool {
	if !c.FakeRanks.Enabled {
		return false
	}
	if c.FakeRanks.Default != nil && c.FakeRanks.Default.hasAny() {
		return true
	}
	for _, o := range c.FakeRanks.Playlists {
		if o != nil && o.hasAny() {
			return true
		}
	}
	if c.FakeRanks.RewardLevels != nil && c.FakeRanks.RewardLevels.hasAny() {
		return true
	}
	return false
}

func (o *FakeRankOverride) hasAny() bool {
	if o == nil {
		return false
	}
	return o.DisplayMMR != nil || o.Mu != nil || o.Sigma != nil ||
		o.Tier != nil || o.Division != nil || o.WinStreak != nil
}

func muFromDisplay(display float64) float64 {
	return (display - fakeRankDisplayOffset) / fakeRankDisplayScale
}

func displayFromMu(mu float64) float64 {
	return mu*fakeRankDisplayScale + fakeRankDisplayOffset
}

func (c SpoofConfig) fakeRankForPlaylist(playlist int) *FakeRankOverride {
	key := strconv.Itoa(playlist)
	if o, ok := c.FakeRanks.Playlists[key]; ok && o != nil && o.hasAny() {
		return o
	}
	if c.FakeRanks.Default != nil && c.FakeRanks.Default.hasAny() {
		return c.FakeRanks.Default
	}
	return nil
}

func (o *FakeRankOverride) resolvedMu() (float64, bool) {
	if o == nil {
		return 0, false
	}
	if o.Mu != nil {
		return *o.Mu, true
	}
	if o.DisplayMMR != nil {
		return muFromDisplay(*o.DisplayMMR), true
	}
	return 0, false
}

func patchWSFakeRanks(frame []byte, c SpoofConfig) ([]byte, bool) {
	if !c.fakeRanksActive() {
		return frame, false
	}
	hdrEnd := bytes.Index(frame, []byte("\r\n\r\n"))
	if hdrEnd < 0 {
		return frame, false
	}
	headers := frame[:hdrEnd]
	body := frame[hdrEnd+4:]
	svc := strings.ToLower(wsHeaderValue(headers, "PsyService"))
	if svc == "" {

		svc = guessSkillService(body)
	}
	var (
		out []byte
		ok  bool
	)
	switch {
	case strings.Contains(svc, "skills/getplayerskill"),
		strings.Contains(svc, "skills/getplayersskills"),
		guessIsGetPlayerSkillBody(body):
		out, ok = patchGetPlayerSkillBody(body, c)
	case strings.Contains(svc, "skills/getskillleaderboardvalueforuser"),
		guessIsLeaderboardValueBody(body):
		out, ok = patchLeaderboardValueBody(body, c)
	default:
		return frame, false
	}
	if !ok {
		return frame, false
	}
	newHeaders := resignWSHeaders(headers, out)
	res := make([]byte, 0, len(newHeaders)+4+len(out))
	res = append(res, newHeaders...)
	res = append(res, '\r', '\n', '\r', '\n')
	res = append(res, out...)
	return res, true
}

func guessSkillService(body []byte) string {
	if guessIsGetPlayerSkillBody(body) {
		return "skills/getplayerskill v1"
	}
	if guessIsLeaderboardValueBody(body) {
		return "skills/getskillleaderboardvalueforuser v1"
	}
	return ""
}

func guessIsGetPlayerSkillBody(body []byte) bool {
	trim := bytes.TrimSpace(body)
	return bytes.Contains(trim, []byte(`"Skills"`)) && bytes.Contains(trim, []byte(`"Mu"`))
}

func guessIsLeaderboardValueBody(body []byte) bool {
	trim := bytes.TrimSpace(body)
	return bytes.Contains(trim, []byte(`"LeaderboardID"`)) &&
		bytes.Contains(trim, []byte(`"bHasSkill"`)) &&
		bytes.Contains(trim, []byte(`"MMR"`))
}

func patchGetPlayerSkillBody(body []byte, c SpoofConfig) ([]byte, bool) {
	var root map[string]json.RawMessage
	if err := json.Unmarshal(body, &root); err != nil {
		return body, false
	}

	resultKey := ""
	var resultObj map[string]json.RawMessage
	if raw, ok := root["Result"]; ok {
		if err := json.Unmarshal(raw, &resultObj); err != nil {
			return body, false
		}
		resultKey = "Result"
	} else {
		resultObj = root
	}

	skillsRaw, ok := resultObj["Skills"]
	if !ok {

		return patchGetPlayersSkillsBody(body, root, resultKey, resultObj, c)
	}

	var skills []map[string]interface{}
	if err := json.Unmarshal(skillsRaw, &skills); err != nil {
		return body, false
	}
	changed := false
	for i := range skills {
		pl := playlistIDFromSkill(skills[i])
		ov := c.fakeRankForPlaylist(pl)
		if ov == nil {
			continue
		}
		if applyRankOverride(skills[i], ov) {
			changed = true
			log.Printf("[fake_ranks] playlist=%d Mu=%.4f display≈%.0f tier=%v div=%v",
				pl, asFloat(skills[i]["Mu"]), displayFromMu(asFloat(skills[i]["Mu"])),
				skills[i]["Tier"], skills[i]["Division"])
		}
	}
	if rl := c.FakeRanks.RewardLevels; rl != nil && rl.hasAny() {
		if applyRewardLevels(resultObj, rl) {
			changed = true
		}
	}
	if !changed {
		return body, false
	}
	newSkills, err := json.Marshal(skills)
	if err != nil {
		return body, false
	}
	resultObj["Skills"] = json.RawMessage(newSkills)
	return remarshalSkillRoot(root, resultKey, resultObj)
}

func patchGetPlayersSkillsBody(
	body []byte,
	root map[string]json.RawMessage,
	resultKey string,
	resultObj map[string]json.RawMessage,
	c SpoofConfig,
) ([]byte, bool) {
	playersRaw, ok := resultObj["Players"]
	if !ok {
		return body, false
	}
	var players []map[string]interface{}
	if err := json.Unmarshal(playersRaw, &players); err != nil {
		return body, false
	}
	changed := false
	for pi := range players {
		skillsAny, ok := players[pi]["Skills"]
		if !ok {
			continue
		}
		skillsSlice, ok := skillsAny.([]interface{})
		if !ok {

			b, _ := json.Marshal(skillsAny)
			var skills []map[string]interface{}
			if json.Unmarshal(b, &skills) != nil {
				continue
			}
			for i := range skills {
				pl := playlistIDFromSkill(skills[i])
				ov := c.fakeRankForPlaylist(pl)
				if ov == nil {
					continue
				}
				if applyRankOverride(skills[i], ov) {
					changed = true
				}
			}
			players[pi]["Skills"] = skills
			continue
		}
		for i := range skillsSlice {
			sm, ok := skillsSlice[i].(map[string]interface{})
			if !ok {
				continue
			}
			pl := playlistIDFromSkill(sm)
			ov := c.fakeRankForPlaylist(pl)
			if ov == nil {
				continue
			}
			if applyRankOverride(sm, ov) {
				changed = true
				skillsSlice[i] = sm
			}
		}
		players[pi]["Skills"] = skillsSlice
	}
	if !changed {
		return body, false
	}
	newPlayers, err := json.Marshal(players)
	if err != nil {
		return body, false
	}
	resultObj["Players"] = json.RawMessage(newPlayers)
	return remarshalSkillRoot(root, resultKey, resultObj)
}

func patchLeaderboardValueBody(body []byte, c SpoofConfig) ([]byte, bool) {
	var root map[string]json.RawMessage
	if err := json.Unmarshal(body, &root); err != nil {
		return body, false
	}
	resultKey := ""
	var resultObj map[string]interface{}
	if raw, ok := root["Result"]; ok {
		if err := json.Unmarshal(raw, &resultObj); err != nil {
			return body, false
		}
		resultKey = "Result"
	} else if err := json.Unmarshal(body, &resultObj); err != nil {
		return body, false
	}

	pl := 0
	if id, ok := resultObj["LeaderboardID"].(string); ok {
		id = strings.TrimPrefix(id, "Skill")
		if n, err := strconv.Atoi(id); err == nil {
			pl = n
		}
	}
	ov := c.fakeRankForPlaylist(pl)
	if ov == nil {

		ov = c.FakeRanks.Default
	}
	if ov == nil || !ov.hasAny() {
		return body, false
	}
	changed := false
	if mu, ok := ov.resolvedMu(); ok {
		resultObj["MMR"] = mu
		changed = true
	}
	if ov.Tier != nil {
		resultObj["Value"] = *ov.Tier
		changed = true
	}
	if !changed {
		return body, false
	}
	resultObj["bHasSkill"] = true
	newResult, err := json.Marshal(resultObj)
	if err != nil {
		return body, false
	}
	if resultKey == "" {
		return newResult, true
	}
	root[resultKey] = json.RawMessage(newResult)
	out, err := json.Marshal(root)
	if err != nil {
		return body, false
	}
	log.Printf("[fake_ranks] leaderboard playlist=%d MMR=%.4f Value=%v", pl, asFloat(resultObj["MMR"]), resultObj["Value"])
	return out, true
}

func remarshalSkillRoot(root map[string]json.RawMessage, resultKey string, resultObj map[string]json.RawMessage) ([]byte, bool) {
	if resultKey != "" {
		b, err := json.Marshal(resultObj)
		if err != nil {
			return nil, false
		}
		root[resultKey] = json.RawMessage(b)
		out, err := json.Marshal(root)
		if err != nil {
			return nil, false
		}
		return out, true
	}
	out, err := json.Marshal(resultObj)
	if err != nil {
		return nil, false
	}
	return out, true
}

func applyRewardLevels(resultObj map[string]json.RawMessage, rl *FakeRewardLevels) bool {
	if rl == nil || !rl.hasAny() {
		return false
	}
	var levels map[string]interface{}
	if raw, ok := resultObj["RewardLevels"]; ok {
		if err := json.Unmarshal(raw, &levels); err != nil {
			levels = map[string]interface{}{}
		}
	} else {
		levels = map[string]interface{}{}
	}
	changed := false
	if rl.SeasonLevel != nil {
		levels["SeasonLevel"] = *rl.SeasonLevel
		changed = true
	}
	if rl.SeasonLevelWins != nil {
		wins := *rl.SeasonLevelWins
		if wins < 0 {
			wins = 0
		} else if wins > 10 {
			wins = 10
		}
		levels["SeasonLevelWins"] = wins
		changed = true
	}
	if !changed {
		return false
	}
	b, err := json.Marshal(levels)
	if err != nil {
		return false
	}
	resultObj["RewardLevels"] = json.RawMessage(b)
	log.Printf("[fake_ranks] reward_levels season=%v wins=%v", levels["SeasonLevel"], levels["SeasonLevelWins"])
	return true
}

func applyRankOverride(skill map[string]interface{}, ov *FakeRankOverride) bool {
	if ov == nil {
		return false
	}
	changed := false
	if mu, ok := ov.resolvedMu(); ok {
		skill["Mu"] = mu
		skill["MMR"] = mu
		changed = true
	}
	if ov.Sigma != nil {
		skill["Sigma"] = *ov.Sigma
		changed = true
	}
	if ov.Tier != nil {
		skill["Tier"] = *ov.Tier
		changed = true
	}
	if ov.Division != nil {
		skill["Division"] = *ov.Division
		changed = true
	}
	if ov.WinStreak != nil {
		skill["WinStreak"] = *ov.WinStreak
		changed = true
	}
	return changed
}

func playlistIDFromSkill(skill map[string]interface{}) int {
	switch v := skill["Playlist"].(type) {
	case float64:
		return int(v)
	case json.Number:
		n, _ := v.Int64()
		return int(n)
	case string:
		n, _ := strconv.Atoi(v)
		return n
	default:
		return 0
	}
}

func asFloat(v interface{}) float64 {
	switch x := v.(type) {
	case float64:
		return x
	case json.Number:
		f, _ := x.Float64()
		return f
	case int:
		return float64(x)
	case int64:
		return float64(x)
	default:
		return 0
	}
}

func fakeRanksSummary(c SpoofConfig) string {
	if !c.fakeRanksActive() {
		return "off"
	}
	parts := []string{}
	if c.FakeRanks.RewardLevels != nil && c.FakeRanks.RewardLevels.SeasonLevel != nil {
		parts = append(parts, fmt.Sprintf("season=%d", *c.FakeRanks.RewardLevels.SeasonLevel))
	}
	if c.FakeRanks.Default != nil {
		if mu, ok := c.FakeRanks.Default.resolvedMu(); ok {
			parts = append(parts, fmt.Sprintf("default display≈%.0f", displayFromMu(mu)))
		} else {
			parts = append(parts, "default(tier/div)")
		}
	}
	for k, o := range c.FakeRanks.Playlists {
		if o == nil {
			continue
		}
		if mu, ok := o.resolvedMu(); ok {
			parts = append(parts, fmt.Sprintf("%s→≈%.0f", k, displayFromMu(mu)))
		} else {
			parts = append(parts, k)
		}
	}
	if len(parts) == 0 {
		return "on"
	}
	return strings.Join(parts, ", ")
}
