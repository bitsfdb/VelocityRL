package main

import (
	"bytes"
	"encoding/base64"
	"encoding/json"
	"log"
	"net/http"
	"net/url"
	"strings"
	"sync/atomic"
)

type NameSpoofConfig struct {
	Enabled     bool   `json:"enabled"`
	DisplayName string `json:"display_name"`

	RealName string `json:"real_name"`

	PlayerID string `json:"player_id"`

	Broker bool `json:"broker"`

	ClassPropName bool `json:"classprop_name"`

	WebSocket bool `json:"websocket"`
	WSEnabled bool `json:"ws_enabled"`

	RewriteWSURL bool `json:"rewrite_ws_url"`

	WSLocalURL string `json:"ws_local_url"`

	OpenSSLTrust bool `json:"openssl_trust"`

	DisablePsyTokenNameID bool `json:"disable_psy_token_nameid"`

	PatchVerifiedPlayerName bool `json:"patch_verified_player_name"`

	ReplaceAllPlayerNames bool `json:"replace_all_player_names"`
}

const defaultWSLocalURL = "http://127.0.0.1:27505/ws/gc2"

var learnedRealPlayerName atomic.Value

var learnedOwnPlayerID atomic.Value

func rememberRealPlayerName(name string) {
	name = strings.TrimSpace(name)
	if name == "" {
		return
	}
	learnedRealPlayerName.Store(name)
	log.Printf("[name_spoof] learned real VerifiedPlayerName=%q", name)
}

func isPlaceholderPlayerID(id string) bool {
	id = strings.TrimSpace(id)
	if id == "" {
		return true
	}
	lower := strings.ToLower(id)
	if strings.Contains(lower, "|temp|") || strings.HasSuffix(lower, "|temp|0") {
		return true
	}
	return false
}

func rememberOwnPlayerID(id string) {
	id = strings.TrimSpace(id)
	if id == "" || isPlaceholderPlayerID(id) {
		return
	}
	if prev, ok := learnedOwnPlayerID.Load().(string); !ok || prev != id {
		learnedOwnPlayerID.Store(id)
		log.Printf("[name_spoof] learned own PlayerID=%q (WS names scoped to this id only)", id)
	}
	persistLearnedPlayerID(id)
}

func (c SpoofConfig) nameSpoofRealName() string {
	if n := strings.TrimSpace(c.NameSpoof.RealName); n != "" {
		return n
	}
	if v := learnedRealPlayerName.Load(); v != nil {
		if s, ok := v.(string); ok {
			return s
		}
	}
	return ""
}

func replaceAllNameOccurrences(body []byte, realName, displayName string) ([]byte, int) {
	if realName == "" || realName == displayName {
		return body, 0
	}
	n := 0
	out := body
	if oldLit, err := json.Marshal(realName); err == nil {
		if newLit, err := json.Marshal(displayName); err == nil {
			if c := bytes.Count(out, oldLit); c > 0 {
				out = bytes.ReplaceAll(out, oldLit, newLit)
				n += c
			}
		}
	}
	old := []byte(realName)
	if c := bytes.Count(out, old); c > 0 {
		out = bytes.ReplaceAll(out, old, []byte(displayName))
		n += c
	}
	return out, n
}

func patchAllNameSpoofs(body []byte, realName, displayName string) ([]byte, int) {
	return replaceAllNameOccurrences(body, realName, displayName)
}

func patchAuthPlayerDisplayName(body []byte, displayName string) ([]byte, bool) {
	return body, false
}

func (c SpoofConfig) psyTokenNameIDActive() bool {
	return false
}

func (c SpoofConfig) patchVerifiedPlayerNameActive() bool {
	return false
}

func patchAllPsyTokensInBody(body []byte, realName, displayName string) ([]byte, int) {
	if realName == "" || realName == displayName {
		return body, 0
	}
	prefix := []byte(`"PsyToken":"`)
	n := 0
	out := body
	searchFrom := 0
	for {
		rel := bytes.Index(out[searchFrom:], prefix)
		if rel < 0 {
			break
		}
		i := searchFrom + rel
		valStart := i + len(prefix)
		valEnd := jsonStringEnd(out, valStart)
		if valEnd < 0 {
			break
		}
		token := string(out[valStart:valEnd])
		newTok, changed := patchJWTNameID(token, realName, displayName)
		if !changed {
			searchFrom = valEnd
			continue
		}
		encoded, ok := jsonStringContents(newTok)
		if !ok {
			searchFrom = valEnd
			continue
		}
		next := append([]byte(nil), out[:valStart]...)
		next = append(next, encoded...)
		next = append(next, out[valEnd:]...)
		out = next
		n++
		log.Printf("[name_spoof] PsyToken JWT nameid %q -> %q", realName, displayName)
		searchFrom = valStart + len(encoded)
	}
	return out, n
}

func patchJWTNameID(token, realName, displayName string) (string, bool) {
	parts := strings.Split(token, ".")
	if len(parts) != 3 {
		return token, false
	}
	payload, err := base64.RawURLEncoding.DecodeString(parts[1])
	if err != nil {
		payload, err = base64.URLEncoding.DecodeString(parts[1])
		if err != nil {
			return token, false
		}
	}
	var claims map[string]interface{}
	if err := json.Unmarshal(payload, &claims); err != nil {
		return token, false
	}
	id, ok := claims["nameid"].(string)
	if !ok || id != realName {
		return token, false
	}
	if id == displayName {
		return token, false
	}
	claims["nameid"] = displayName
	newPayload, err := json.Marshal(claims)
	if err != nil {
		return token, false
	}
	newB64 := base64.RawURLEncoding.EncodeToString(newPayload)
	return parts[0] + "." + newB64 + "." + parts[2], true
}

func (c SpoofConfig) nameSpoofReplaceActive() bool {
	if c.nameSpoofDisplay() == "" {
		return false
	}
	if c.NameSpoof.ReplaceAllPlayerNames {
		return true
	}
	if c.nameSpoofRealName() != "" {
		return true
	}
	if c.nameSpoofOwnPlayerID() != "" &&
		(c.nameSpoofBrokerActive() || c.NameSpoof.ClassPropName || c.wsHostsMITMActive()) {
		return true
	}
	return false
}

func (c SpoofConfig) nameSpoofReplaceAllPlayerNames() bool {
	return c.NameSpoof.ReplaceAllPlayerNames && c.nameSpoofDisplay() != ""
}

func (c SpoofConfig) nameSpoofOwnPlayerID() string {
	if n := strings.TrimSpace(c.NameSpoof.PlayerID); n != "" && !isPlaceholderPlayerID(n) {
		return n
	}
	if v := learnedOwnPlayerID.Load(); v != nil {
		if s, ok := v.(string); ok && s != "" && !isPlaceholderPlayerID(s) {
			return s
		}
	}
	return ""
}

func learnIdentityFromAuthRequest(body []byte) {}

func (c SpoofConfig) nameSpoofBrokerActive() bool {
	return c.NameSpoof.Broker && c.nameSpoofDisplay() != ""
}

func (c SpoofConfig) authPlayerPatchingActive() bool {
	return false
}

func (c SpoofConfig) nameSpoofRewriteWSActive() bool {
	// 2.0 shipping: always rewrite AuthPlayer PerConURL/PerConURLv2 to local broker WS.
	return true
}

func (c SpoofConfig) wsSpoofNeedsMITM() bool {
	return c.fakeRanksActive()
}

func (c SpoofConfig) wsHostsMITMActive() bool {
	// Broker PerCon rewrite path never adds ws.rlpp hosts.
	if c.nameSpoofRewriteWSActive() {
		return false
	}
	return c.wsSpoofNeedsMITM() || c.nameSpoofWSActive()
}

func (c SpoofConfig) psyNetURLBrokerRewriteActive() bool {
	// Always rewrite PsyNetUrl -> 127.0.0.1 broker while the proxy is running.
	// AuthPlayer then hits the broker and PerCon becomes http://127.0.0.1:27505/ws/...
	return true
}

func (c SpoofConfig) wsLocalURL() string {
	if u := strings.TrimSpace(c.NameSpoof.WSLocalURL); u != "" {
		return u
	}
	return defaultWSLocalURL
}

func (c SpoofConfig) nameSpoofOpenSSLTrustActive() bool {
	return c.nameSpoofWSActive() && c.NameSpoof.OpenSSLTrust
}

func (c SpoofConfig) nameSpoofActive() bool {
	return c.NameSpoof.Enabled && c.nameSpoofDisplay() != ""
}

func (c SpoofConfig) nameSpoofWSActive() bool {
	if c.nameSpoofDisplay() == "" {
		return false
	}
	if !(c.NameSpoof.WebSocket || c.NameSpoof.WSEnabled) {
		return false
	}
	return c.NameSpoof.Enabled || c.NameSpoof.Broker
}

func (c SpoofConfig) nameSpoofDisplay() string {
	if n := strings.TrimSpace(c.NameSpoof.DisplayName); n != "" {
		return n
	}

	return strings.TrimSpace(c.CustomName)
}

func allowedMITMHost(host string, c SpoofConfig) bool {
	h := strings.ToLower(host)
	if strings.Contains(h, "config.psynet.gg") {
		return true
	}
	if c.nameSpoofActive() && strings.Contains(h, "api.rlpp.psynet.gg") {
		return true
	}
	if c.wsHostsMITMActive() && strings.Contains(h, "ws.rlpp.psynet.gg") {
		return true
	}
	return false
}

func isAuthPlayerPath(path string) bool {
	p := strings.ToLower(path)
	return strings.Contains(p, "/rpc/auth/authplayer")
}

func replaceJSONStringField(body []byte, key, newValue string) ([]byte, bool) {
	encoded, ok := jsonStringContents(newValue)
	if !ok {
		return body, false
	}
	prefix := []byte(`"` + key + `":"`)
	i := bytes.Index(body, prefix)
	if i < 0 {
		return body, false
	}
	valStart := i + len(prefix)
	j := jsonStringEnd(body, valStart)
	if j < 0 {
		return body, false
	}
	if bytes.Equal(body[valStart:j], encoded) {
		return body, false
	}
	out := append([]byte(nil), body[:valStart]...)
	out = append(out, encoded...)
	out = append(out, body[j:]...)
	return out, true
}

func patchAuthPlayerRequest(body []byte, displayName string) ([]byte, bool) {
	return body, false
}

func patchAuthPlayerResponse(body []byte, displayName string) ([]byte, bool) {
	return body, false
}

func extractJSONStringField(body []byte, key string) (string, bool) {
	prefix := []byte(`"` + key + `":"`)
	i := bytes.Index(body, prefix)
	if i < 0 {
		return "", false
	}
	valStart := i + len(prefix)
	j := jsonStringEnd(body, valStart)
	if j < 0 {
		return "", false
	}
	raw := append([]byte{'"'}, body[valStart:j]...)
	raw = append(raw, '"')
	var s string
	if err := json.Unmarshal(raw, &s); err != nil {
		return string(body[valStart:j]), true
	}
	return s, true
}

func localPerConURLv1(localURLv2 string) string {
	u, err := url.Parse(strings.TrimSpace(localURLv2))
	if err != nil || u.Scheme == "" || u.Host == "" {
		return "http://127.0.0.1:27505/ws/gc?PsyConnectionType=Player"
	}
	u.Path = "/ws/gc"
	u.RawQuery = "PsyConnectionType=Player"
	u.Fragment = ""
	return u.String()
}

func patchAuthPlayerWSURL(body []byte, localURLv2 string) ([]byte, bool) {
	localURLv2 = strings.TrimSpace(localURLv2)
	if localURLv2 == "" {
		return body, false
	}
	localURLv1 := localPerConURLv1(localURLv2)
	changed := false
	out := body
	if next, did := replaceJSONStringField(out, "PerConURLv2", localURLv2); did {
		out = next
		changed = true
	}
	if next, did := replaceJSONStringField(out, "PerConURL", localURLv1); did {
		out = next
		changed = true
	}
	return out, changed
}

func resignRPCHeader(hdr http.Header, body []byte) {
	psyTime := hdr.Get("PsyTime")
	sig := resign(psyTime, body)
	hdr.Set("PsySig", sig)
	hdr.Del("Psysignature")
}

func setRPCSig(resp *http.Response, body []byte) {
	psyTime := resp.Header.Get("PsyTime")
	sig := resign(psyTime, body)
	resp.Header.Set("PsySig", sig)
	resp.Header.Del("Psysignature")
}

func resignRequestSig(req *http.Request, body []byte) {
	if req == nil {
		return
	}
	got := req.Header.Get("PsySig")
	if got == "" {
		got = req.Header.Get("Psysignature")
	}
	if got == "" {
		return
	}
	psyTime := req.Header.Get("PsyTime")
	sig := resign(psyTime, body)
	req.Header.Set("PsySig", sig)
	req.Header.Del("Psysignature")
}

func maybePatchNameSpoofHTTP(resp *http.Response, body []byte) ([]byte, bool) {

	return body, false
}
