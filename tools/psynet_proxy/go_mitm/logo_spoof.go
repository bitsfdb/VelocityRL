package main

import (
	"bytes"
	"encoding/json"
	"log"
	"strings"
)

const DefaultSeason23LogoURL = "https://rl-cdn.psyonix.com/LogoImages/S23/rl_season-logo_23_EN_1.png"

// PsyNet / UE field names seen on DynamicLogosConfig (LogoURL is the live one).
var dynamicLogoURLKeys = []string{"LogoURL", "SeasonLogo", "SeasonLogoURL", "LogoImageURL", "DynamicLogoURL"}

type LogoSpoofConfig struct {
	Enabled bool   `json:"enabled"`
	LogoURL string `json:"logo_url"`
}

func (c SpoofConfig) logoSpoofActive() bool {
	return c.LogoSpoof.Enabled && strings.TrimSpace(c.LogoSpoof.LogoURL) != ""
}

func patchDynamicLogoURLRaw(body []byte, c SpoofConfig) ([]byte, bool) {
	return patchDynamicLogoURLRawInject(body, c, false)
}

func patchDynamicLogoURLRawInject(body []byte, c SpoofConfig, allowInject bool) ([]byte, bool) {
	if !c.logoSpoofActive() {
		return body, false
	}
	want := strings.TrimSpace(c.LogoSpoof.LogoURL)
	escapeSlash := bytes.Contains(body, []byte(`\/`))
	encoded, okEnc := jsonEncodeStringInner(want, escapeSlash)
	if !okEnc {
		log.Printf("[logo_spoof] LogoURL could not be JSON-encoded")
		return body, false
	}

	obj, objStart, objEnd, ok := jsonNamedObject(body, "DynamicLogosConfig")
	if !ok {
		if !allowInject {
			log.Printf("[logo_spoof] DynamicLogosConfig not found")
			return body, false
		}
		out, inserted := injectDynamicLogosConfig(body, encoded)
		if !inserted {
			log.Printf("[logo_spoof] DynamicLogosConfig not found and inject failed")
			return body, false
		}
		log.Printf("[logo_spoof] inserted DynamicLogosConfig.LogoURL -> %q (%d -> %d bytes)", want, len(body), len(out))
		return out, true
	}

	out := body
	changed := false

	if next, did := forceJSONBoolInObject(out, obj, objStart, "bUseDynamicLogos", true); did {
		out = next
		changed = true
		obj, objStart, objEnd, ok = jsonNamedObject(out, "DynamicLogosConfig")
		if !ok {
			return body, false
		}
	} else if !jsonObjectHasKey(obj, "bUseDynamicLogos") {
		out = insertBeforeObjectClose(out, objEnd, []byte(`,"bUseDynamicLogos":true`))
		changed = true
		obj, objStart, objEnd, ok = jsonNamedObject(out, "DynamicLogosConfig")
		if !ok {
			return body, false
		}
	}

	rewroteURL := false
	foundURL := false
	for _, key := range dynamicLogoURLKeys {
		obj, objStart, objEnd, ok = jsonNamedObject(out, "DynamicLogosConfig")
		if !ok {
			return body, false
		}
		valStart, valEnd, ok := jsonStringFieldInObject(out, obj, objStart, key)
		if !ok {
			continue
		}
		foundURL = true
		curInner := out[valStart:valEnd]
		if cur, okU := jsonUnquoteInner(curInner); okU && cur == want {
			continue
		}
		useSlash := bytes.Contains(curInner, []byte(`\/`)) || escapeSlash
		enc, okE := jsonEncodeStringInner(want, useSlash)
		if !okE {
			log.Printf("[logo_spoof] %s could not be JSON-encoded", key)
			return body, false
		}
		patched := append([]byte(nil), out[:valStart]...)
		patched = append(patched, enc...)
		patched = append(patched, out[valEnd:]...)
		out = patched
		changed = true
		rewroteURL = true
	}

	if !foundURL {
		enc, okE := jsonEncodeStringInner(want, escapeSlash)
		if !okE {
			return body, false
		}
		snippet := append([]byte(`,"LogoURL":"`), enc...)
		snippet = append(snippet, '"')
		out = insertBeforeObjectClose(out, objEnd, snippet)
		changed = true
		rewroteURL = true
		log.Printf("[logo_spoof] inserted DynamicLogosConfig.LogoURL -> %q", want)
	}

	if !changed {
		return body, false
	}
	if rewroteURL {
		log.Printf("[logo_spoof] DynamicLogosConfig.LogoURL -> %q (%d -> %d bytes)", want, len(body), len(out))
	} else {
		log.Printf("[logo_spoof] bUseDynamicLogos forced true (%d -> %d bytes)", len(body), len(out))
	}
	return out, true
}

func injectDynamicLogosConfig(body []byte, encodedLogo []byte) ([]byte, bool) {
	close := bytes.LastIndexByte(body, '}')
	if close < 0 {
		return body, false
	}
	block := []byte(`,"DynamicLogosConfig":{"Class":"DynamicLogosConfig_TA","bUseDynamicLogos":true,"LogoURL":"`)
	block = append(block, encodedLogo...)
	block = append(block, '"', '}')
	out := append([]byte(nil), body[:close]...)
	out = append(out, block...)
	out = append(out, body[close:]...)
	return out, true
}

func insertBeforeObjectClose(body []byte, objEnd int, snippet []byte) []byte {
	if objEnd <= 0 || objEnd > len(body) {
		return body
	}
	close := objEnd - 1
	if body[close] != '}' {
		return body
	}
	out := append([]byte(nil), body[:close]...)
	out = append(out, snippet...)
	out = append(out, body[close:]...)
	return out
}

func skipJSONSpace(b []byte, i int) int {
	for i < len(b) && (b[i] == ' ' || b[i] == '\t' || b[i] == '\n' || b[i] == '\r') {
		i++
	}
	return i
}

func jsonNamedObject(body []byte, key string) (obj []byte, start, end int, ok bool) {
	needle := []byte(`"` + key + `"`)
	from := 0
	for {
		at := bytes.Index(body[from:], needle)
		if at < 0 {
			return nil, 0, 0, false
		}
		at += from
		i := skipJSONSpace(body, at+len(needle))
		if i >= len(body) || body[i] != ':' {
			from = at + 1
			continue
		}
		i = skipJSONSpace(body, i+1)
		if i >= len(body) || body[i] != '{' {
			from = at + 1
			continue
		}
		start = i
		closeAt := scanObjectEnd(body, start)
		if closeAt < 0 {
			return nil, 0, 0, false
		}
		end = closeAt + 1
		return body[start:end], start, end, true
	}
}

func jsonObjectHasKey(obj []byte, key string) bool {
	needle := []byte(`"` + key + `"`)
	from := 0
	for {
		k := bytes.Index(obj[from:], needle)
		if k < 0 {
			return false
		}
		k += from
		i := skipJSONSpace(obj, k+len(needle))
		if i < len(obj) && obj[i] == ':' {
			return true
		}
		from = k + 1
	}
}

func jsonStringFieldInObject(body, obj []byte, objStart int, key string) (valStart, valEnd int, ok bool) {
	needle := []byte(`"` + key + `"`)
	from := 0
	for {
		k := bytes.Index(obj[from:], needle)
		if k < 0 {
			return 0, 0, false
		}
		k += from
		i := skipJSONSpace(obj, k+len(needle))
		if i >= len(obj) || obj[i] != ':' {
			from = k + 1
			continue
		}
		i = skipJSONSpace(obj, i+1)
		if i >= len(obj) || obj[i] != '"' {
			return 0, 0, false
		}
		valStart = objStart + i + 1
		valEnd = jsonStringEnd(body, valStart)
		if valEnd < 0 {
			return 0, 0, false
		}
		return valStart, valEnd, true
	}
}

func jsonEncodeStringInner(s string, escapeSlash bool) ([]byte, bool) {
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetEscapeHTML(false)
	if err := enc.Encode(s); err != nil {
		return nil, false
	}
	b := bytes.TrimSpace(buf.Bytes())
	if len(b) < 2 || b[0] != '"' || b[len(b)-1] != '"' {
		return nil, false
	}
	inner := b[1 : len(b)-1]
	if escapeSlash {
		inner = bytes.ReplaceAll(inner, []byte("/"), []byte(`\/`))
	}
	return inner, true
}

func jsonUnquoteInner(inner []byte) (string, bool) {
	wrapped := make([]byte, 0, len(inner)+2)
	wrapped = append(wrapped, '"')
	wrapped = append(wrapped, inner...)
	wrapped = append(wrapped, '"')
	var s string
	if err := json.Unmarshal(wrapped, &s); err != nil {
		return "", false
	}
	return s, true
}

// forceJSONBoolInObject sets "key": true/false inside a JSON object slice of body.
// Handles both JSON booleans and quoted "true"/"false" (UE mixed styles).
func forceJSONBoolInObject(body, obj []byte, objStart int, key string, want bool) ([]byte, bool) {
	needle := []byte(`"` + key + `"`)
	from := 0
	for {
		k := bytes.Index(obj[from:], needle)
		if k < 0 {
			return body, false
		}
		k += from
		i := skipJSONSpace(obj, k+len(needle))
		if i >= len(obj) || obj[i] != ':' {
			from = k + 1
			continue
		}
		i = skipJSONSpace(obj, i+1)
		if i >= len(obj) {
			return body, false
		}
		abs := objStart + i
		wantBytes := []byte("false")
		if want {
			wantBytes = []byte("true")
		}
		end := abs
		if abs < len(body) && body[abs] == '"' {
			if abs+6 <= len(body) && bytes.Equal(body[abs:abs+6], []byte(`"true"`)) {
				end = abs + 6
			} else if abs+7 <= len(body) && bytes.Equal(body[abs:abs+7], []byte(`"false"`)) {
				end = abs + 7
			} else {
				return body, false
			}
		} else if abs+4 <= len(body) && bytes.Equal(body[abs:abs+4], []byte("true")) {
			end = abs + 4
		} else if abs+5 <= len(body) && bytes.Equal(body[abs:abs+5], []byte("false")) {
			end = abs + 5
		} else {
			return body, false
		}
		if bytes.Equal(body[abs:end], wantBytes) {
			return body, false
		}
		out := append([]byte(nil), body[:abs]...)
		out = append(out, wantBytes...)
		out = append(out, body[end:]...)
		return out, true
	}
}
