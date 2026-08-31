package main

import (
	"bytes"
	"encoding/base64"
	"encoding/json"
	"log"
	"strings"
)

func logNameSwap(source, ctx, field, from, to string) {
	from = strings.TrimSpace(from)
	to = strings.TrimSpace(to)
	if from == to {
		return
	}
	if ctx != "" {
		log.Printf("[name_swap] src=%s ctx=%s field=%s %q -> %q", source, ctx, field, from, to)
		return
	}
	log.Printf("[name_swap] src=%s field=%s %q -> %q", source, field, from, to)
}

func decodeJSONStringBytes(raw []byte) string {
	wrapped := append([]byte{'"'}, raw...)
	wrapped = append(wrapped, '"')
	var s string
	if err := json.Unmarshal(wrapped, &s); err != nil {
		return string(raw)
	}
	return s
}

func wsSwapContext(dir string, headers, jsonBody []byte) string {
	parts := make([]string, 0, 5)
	if dir != "" {
		parts = append(parts, "dir="+dir)
	}
	if v := wsHeaderValue(headers, "PsyService"); v != "" {
		parts = append(parts, "svc="+v)
	}
	if v := wsHeaderValue(headers, "PsyResponseID"); v != "" {
		parts = append(parts, "resp="+v)
	}
	if v := wsHeaderValue(headers, "PsyRequestID"); v != "" {
		parts = append(parts, "req="+v)
	}
	if v := wsHeaderValue(headers, "PsyConnectionID"); v != "" {
		parts = append(parts, "conn="+v)
	}
	if len(jsonBody) > 0 {
		if mt, ok := extractJSONStringField(jsonBody, "MessageType"); ok && mt != "" {
			parts = append(parts, "msg="+mt)
		}
	}
	return strings.Join(parts, " ")
}

func replaceJSONStringFieldAudited(body []byte, key, newValue, source, ctx string) ([]byte, bool) {
	old, had := extractJSONStringField(body, key)
	out, ok := replaceJSONStringField(body, key, newValue)
	if ok && had {
		logNameSwap(source, ctx, key, old, newValue)
	} else if ok {
		logNameSwap(source, ctx, key, "?", newValue)
	}
	return out, ok
}

func replaceJSONStringFieldAllAudited(body []byte, key, newValue, source, ctx string) ([]byte, int) {
	encoded, ok := jsonStringContents(newValue)
	if !ok {
		return body, 0
	}
	prefix := []byte(`"` + key + `":"`)
	count := 0
	searchFrom := 0
	for {
		rel := bytes.Index(body[searchFrom:], prefix)
		if rel < 0 {
			break
		}
		i := searchFrom + rel
		valStart := i + len(prefix)
		j := jsonStringEnd(body, valStart)
		if j < 0 {
			break
		}
		if !bytes.Equal(body[valStart:j], encoded) {
			oldVal := decodeJSONStringBytes(body[valStart:j])
			logNameSwap(source, ctx, key, oldVal, newValue)
			out := append([]byte(nil), body[:valStart]...)
			out = append(out, encoded...)
			out = append(out, body[j:]...)
			body = out
			count++
			searchFrom = valStart + len(encoded)
		} else {
			searchFrom = j
		}
	}
	return body, count
}

func replaceJSONStringFieldAllWhenAudited(body []byte, key, oldValue, newValue, source, ctx string) ([]byte, int) {
	oldEnc, okOld := jsonStringContents(oldValue)
	newEnc, okNew := jsonStringContents(newValue)
	if !okOld || !okNew || oldValue == newValue {
		return body, 0
	}
	prefix := []byte(`"` + key + `":"`)
	count := 0
	searchFrom := 0
	for {
		rel := bytes.Index(body[searchFrom:], prefix)
		if rel < 0 {
			break
		}
		i := searchFrom + rel
		valStart := i + len(prefix)
		j := jsonStringEnd(body, valStart)
		if j < 0 {
			break
		}
		if bytes.Equal(body[valStart:j], oldEnc) {
			oldVal := decodeJSONStringBytes(body[valStart:j])
			logNameSwap(source, ctx, key, oldVal, newValue)
			out := append([]byte(nil), body[:valStart]...)
			out = append(out, newEnc...)
			out = append(out, body[j:]...)
			body = out
			count++
			searchFrom = valStart + len(newEnc)
		} else {
			searchFrom = j
		}
	}
	return body, count
}

var ownIDKeys = []string{
	"PlayerID",
	"UserID",
	"FromUserID",
	"ForUserID",
	"FromEpicUserID",
	"PlayerId",
}

func findEnclosingObject(body []byte, pos int) (start, end int) {
	if pos < 0 || pos >= len(body) {
		return -1, -1
	}
	for start := pos; start >= 0; start-- {
		if body[start] != '{' {
			continue
		}
		closeAt := scanObjectEnd(body, start)
		if closeAt >= pos {
			return start, closeAt + 1
		}
	}
	return -1, -1
}

func objectRangePatched(ranges [][2]int, start, end int) bool {
	for _, r := range ranges {
		if r[0] == start && r[1] == end {
			return true
		}
	}
	return false
}

func patchNameFieldsForOwnID(body []byte, ownID, displayName, source, ctx string) ([]byte, int) {
	if ownID == "" || displayName == "" {
		return body, 0
	}
	n := 0
	out := body
	var patched [][2]int
	for _, idKey := range ownIDKeys {
		needle := []byte(`"` + idKey + `":"` + ownID + `"`)
		searchFrom := 0
		for {
			rel := bytes.Index(out[searchFrom:], needle)
			if rel < 0 {
				break
			}
			abs := searchFrom + rel
			objStart, objEnd := findEnclosingObject(out, abs)
			if objStart < 0 {
				searchFrom = abs + 1
				continue
			}
			if objectRangePatched(patched, objStart, objEnd) {
				searchFrom = abs + 1
				continue
			}
			seg := out[objStart:objEnd]
			segOut := seg
			segN := 0
			for _, nameKey := range wsNameKeys {
				next, c := replaceJSONStringFieldAllAudited(segOut, nameKey, displayName, source, ctx)
				if c > 0 {
					segOut = next
					segN += c
				}
			}
			if segN > 0 {
				newOut := append([]byte(nil), out[:objStart]...)
				newOut = append(newOut, segOut...)
				newOut = append(newOut, out[objEnd:]...)
				out = newOut
				n += segN
				patched = append(patched, [2]int{objStart, objStart + len(segOut)})

				searchFrom = objStart + len(segOut)
			} else {

				patched = append(patched, [2]int{objStart, objEnd})
				searchFrom = objEnd
			}
		}
	}
	return out, n
}

func patchBase64ContentNameFields(body []byte, displayName, realName, ownID, ctx string) ([]byte, int) {
	if displayName == "" {
		return body, 0
	}
	prefix := []byte(`"Content":"`)
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
		rawB64 := out[valStart:valEnd]
		decoded, err := base64.StdEncoding.DecodeString(string(rawB64))
		if err != nil {
			decoded, err = base64.RawStdEncoding.DecodeString(string(rawB64))
		}
		searchFrom = valEnd
		if err != nil || len(decoded) == 0 {
			continue
		}
		jsonStart := bytes.IndexByte(decoded, '{')
		if jsonStart < 0 {
			continue
		}
		head := append([]byte(nil), decoded[:jsonStart]...)
		jsonPart := decoded[jsonStart:]
		patched, c := patchJSONNameFields(jsonPart, displayName, realName, ownID, false, ctx+" b64_content")
		if c == 0 {
			continue
		}
		newDecoded := append(head, patched...)
		newB64 := base64.StdEncoding.EncodeToString(newDecoded)
		encoded, ok := jsonStringContents(newB64)
		if !ok {
			continue
		}
		next := append([]byte(nil), out[:valStart]...)
		next = append(next, encoded...)
		next = append(next, out[valEnd:]...)
		out = next
		n += c
		logNameSwap("ws_party_b64", ctx, "Content", "embedded_json", displayName)
		searchFrom = valStart + len(encoded)
	}
	return out, n
}

func scrubRealNameAudited(body []byte, realName, displayName, source, ctx string) ([]byte, int) {
	if realName == "" || realName == displayName {
		return body, 0
	}
	n := 0
	out := body
	for _, key := range wsNameKeys {
		next, c := replaceJSONStringFieldAllWhenAudited(out, key, realName, displayName, source, ctx)
		if c > 0 {
			out = next
			n += c
		}
	}
	if oldLit, err := json.Marshal(realName); err == nil {
		if newLit, err := json.Marshal(displayName); err == nil {
			if c := bytes.Count(out, oldLit); c > 0 {
				logNameSwap(source, ctx, "json_literal", realName, displayName)
				out = bytes.ReplaceAll(out, oldLit, newLit)
				n += c
			}
		}
	}
	old := []byte(realName)
	if c := bytes.Count(out, old); c > 0 {
		logNameSwap(source, ctx, "bare_substring", realName, displayName)
		out = bytes.ReplaceAll(out, old, []byte(displayName))
		n += c
	}
	return out, n
}
