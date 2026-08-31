package main

import (
	"bytes"
	"encoding/json"
	"log"
)

func patchTitleCatalogRaw(body []byte, c SpoofConfig) ([]byte, bool) {
	swaps := c.titleSwaps()
	if len(swaps) == 0 {
		return body, false
	}
	out := body
	changed := false
	for _, s := range swaps {
		next, ok := patchOneTitleCatalogRaw(out, s)
		if ok {
			out = next
			changed = true
		}
	}
	if !changed {
		return body, false
	}
	return out, true
}

func patchOneTitleCatalogRaw(body []byte, c SpoofConfig) ([]byte, bool) {
	equip := c.equipTitleID()
	if equip == "" {
		return body, false
	}

	out := body
	changed := false

	text := c.wantText()
	if text == "" {
		if display := c.displayTitleID(); display != "" {
			var ok bool
			text, ok = rawTitleText(out, display)
			if !ok {
				log.Printf("[patch] catalog raw: display %q not found", display)
			}
		}
	}
	if text != "" {
		if out2, ok := replaceEquipText(out, equip, text); ok {
			out = out2
			changed = true
			log.Printf("[patch] catalog raw: %q Text -> %q (%d bytes)", equip, text, len(out))
		}
	} else {
		log.Printf("[patch] catalog raw: set custom_text or display_title_id")
	}

	cat := c.wantCategory()
	if cat == "" {
		if display := c.displayTitleID(); display != "" {
			if copied, ok := rawTitleCategory(out, display); ok {
				cat = copied
			}
		}
	}
	if cat != "" {
		if out2, okCat := replaceEquipCategory(out, equip, cat); okCat {
			out = out2
			changed = true
			log.Printf("[patch] catalog raw: %q Category -> %q (%d bytes)", equip, cat, len(out))
		} else {
			log.Printf("[patch] catalog raw: %q Category unchanged", equip)
		}
	} else {
		log.Printf("[patch] catalog raw: no category — glow stays default")
	}

	if !changed {
		return body, false
	}
	return out, true
}

func titleObject(body []byte, id string) (obj []byte, start, end int, ok bool) {
	idPat := []byte(`"ID":"` + id + `"`)
	idAt := bytes.Index(body, idPat)
	if idAt < 0 {
		return nil, 0, 0, false
	}
	start = idAt
	for start > 0 && body[start] != '{' {
		start--
	}
	if body[start] != '{' {
		return nil, 0, 0, false
	}
	closeAt := scanObjectEnd(body, start)
	if closeAt < 0 {
		return nil, 0, 0, false
	}
	end = closeAt + 1
	return body[start:end], start, end, true
}

func scanObjectEnd(body []byte, start int) int {
	inStr := false
	esc := false
	depth := 0
	for i := start; i < len(body); i++ {
		c := body[i]
		if inStr {
			if esc {
				esc = false
				continue
			}
			if c == '\\' {
				esc = true
				continue
			}
			if c == '"' {
				inStr = false
			}
			continue
		}
		switch c {
		case '"':
			inStr = true
		case '{':
			depth++
		case '}':
			depth--
			if depth == 0 {
				return i
			}
		}
	}
	return -1
}

func jsonSafeIdent(s string) bool {
	if s == "" {
		return false
	}
	for i := 0; i < len(s); i++ {
		c := s[i]
		if c == '"' || c == '\\' || c < 0x20 {
			return false
		}
	}
	return true
}

func jsonStringContents(s string) ([]byte, bool) {
	b, err := json.Marshal(s)
	if err != nil || len(b) < 2 || b[0] != '"' || b[len(b)-1] != '"' {
		return nil, false
	}
	return b[1 : len(b)-1], true
}

func jsonStringEnd(body []byte, valStart int) int {
	esc := false
	for j := valStart; j < len(body); j++ {
		c := body[j]
		if esc {
			esc = false
			continue
		}
		if c == '\\' {
			esc = true
			continue
		}
		if c == '"' {
			return j
		}
	}
	return -1
}

func replaceEquipText(body []byte, equipID, newText string) ([]byte, bool) {
	encoded, okEnc := jsonStringContents(newText)
	if !okEnc {
		log.Printf("[patch] catalog raw: Text could not be JSON-encoded")
		return body, false
	}

	prefix := []byte(`"ID":"` + equipID + `","Text":"`)
	i := bytes.Index(body, prefix)
	valStart := -1
	if i >= 0 {
		valStart = i + len(prefix)
	} else {
		obj, objStart, _, ok := titleObject(body, equipID)
		if !ok {
			log.Printf("[patch] catalog raw: equip %q not found", equipID)
			return body, false
		}
		tkey := []byte(`"Text":"`)
		k := bytes.Index(obj, tkey)
		if k < 0 {
			log.Printf("[patch] catalog raw: no Text field on %q", equipID)
			return body, false
		}
		valStart = objStart + k + len(tkey)
	}

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

func replaceEquipCategory(body []byte, equipID, newCat string) ([]byte, bool) {
	if !jsonSafeIdent(newCat) {
		return body, false
	}
	obj, objStart, _, ok := titleObject(body, equipID)
	if !ok {
		return body, false
	}

	ckey := []byte(`"Category":"`)
	k := bytes.Index(obj, ckey)
	if k < 0 {
		return insertEquipCategory(body, objStart, obj, equipID, newCat)
	}

	valStart := objStart + k + len(ckey)
	j := jsonStringEnd(body, valStart)
	if j < 0 {
		return body, false
	}
	if bytes.Equal(body[valStart:j], []byte(newCat)) {
		return body, false
	}
	out := append([]byte(nil), body[:valStart]...)
	out = append(out, []byte(newCat)...)
	out = append(out, body[j:]...)
	return out, true
}

func insertEquipCategory(body []byte, objStart int, obj []byte, equipID, newCat string) ([]byte, bool) {
	idPat := []byte(`"ID":"` + equipID + `"`)
	k := bytes.Index(obj, idPat)
	if k < 0 {
		return body, false
	}
	insertAt := objStart + k + len(idPat)
	frag := []byte(`,"Category":"` + newCat + `"`)
	out := append([]byte(nil), body[:insertAt]...)
	out = append(out, frag...)
	out = append(out, body[insertAt:]...)
	return out, true
}

func rawTitleText(body []byte, id string) (string, bool) {
	obj, _, _, ok := titleObject(body, id)
	if !ok {
		return "", false
	}
	tkey := []byte(`"Text":"`)
	k := bytes.Index(obj, tkey)
	if k < 0 {
		return "", false
	}
	openQuote := k + len(`"Text":`)
	end := jsonStringEnd(obj, openQuote+1)
	if end < 0 {
		return "", false
	}
	var s string
	if json.Unmarshal(obj[openQuote:end+1], &s) != nil {
		return "", false
	}
	return s, true
}

func rawTitleCategory(body []byte, id string) (string, bool) {
	obj, _, _, ok := titleObject(body, id)
	if !ok {
		return "", false
	}
	ckey := []byte(`"Category":"`)
	k := bytes.Index(obj, ckey)
	if k < 0 {
		return "", false
	}
	chunk := obj[k+len(ckey):]
	end := bytes.IndexByte(chunk, '"')
	if end < 0 {
		return "", false
	}
	return string(chunk[:end]), true
}

func patchTitleCatalogRegex(body []byte, c SpoofConfig) ([]byte, bool) {
	swaps := c.titleSwaps()
	if len(swaps) == 0 {
		return body, false
	}
	out := body
	changed := false
	for _, s := range swaps {
		next, ok := patchOneTitleCatalogRegex(out, s)
		if ok {
			out = next
			changed = true
		}
	}
	if !changed {
		return body, false
	}
	return out, true
}

func patchOneTitleCatalogRegex(body []byte, c SpoofConfig) ([]byte, bool) {
	equip := c.equipTitleID()
	text := c.wantText()
	if text == "" {
		if d := c.displayTitleID(); d != "" {
			text, _ = rawTitleText(body, d)
		}
	}
	if equip == "" || text == "" {
		return body, false
	}
	out, ok := replaceEquipText(body, equip, text)
	if ok {
		log.Printf("[patch] catalog regex: %q Text -> %q (%d -> %d bytes)", equip, text, len(body), len(out))
	}
	return out, ok
}
