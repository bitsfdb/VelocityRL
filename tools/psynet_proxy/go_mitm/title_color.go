package main

import (
	"bytes"
	"encoding/json"
	"log"
	"strings"
)

type TitleColor struct {
	Color     string `json:"color"`
	GlowColor string `json:"glow_color"`
}

func (tc TitleColor) valid() bool {
	return isHex6(tc.Color) && (tc.GlowColor == "" || isHex6(tc.GlowColor))
}

func isHex6(s string) bool {
	if len(s) != 6 {
		return false
	}
	for i := 0; i < len(s); i++ {
		c := s[i]
		if !(c >= '0' && c <= '9') && !(c >= 'a' && c <= 'f') && !(c >= 'A' && c <= 'F') {
			return false
		}
	}
	return true
}

func titleCustomCategoryID(equipID, text string) string {
	base := equipID
	if base == "" {
		base = "custom"
	}
	id := "vrl_custom_" + sanitizeCategoryPart(base)
	if text != "" {

		_ = text
	}
	return id
}

func sanitizeCategoryPart(s string) string {
	out := make([]byte, 0, len(s))
	for i := 0; i < len(s); i++ {
		c := s[i]
		switch {
		case c >= 'a' && c <= 'z', c >= 'A' && c <= 'Z', c >= '0' && c <= '9', c == '_':
			out = append(out, c)
		default:
			out = append(out, '_')
		}
	}
	if len(out) == 0 {
		return "title"
	}
	return string(out)
}

func upsertTitleCategoryRaw(body []byte, catID, color, glowColor string) ([]byte, bool) {
	objStart, objEnd, ok := playerTitleConfigObject(body)
	if !ok {
		log.Printf("[title_color] PlayerTitleConfig not found")
		return body, false
	}
	arrStart, arrEnd, ok := categoriesArrayBounds(body, objStart, objEnd)
	if !ok {
		log.Printf("[title_color] Categories array not found")
		return body, false
	}

	def, encErr := json.Marshal(map[string]string{
		"ID":        catID,
		"Color":     color,
		"GlowColor": glowColor,
	})
	if encErr != nil {
		return body, false
	}
	_ = def

	var b bytes.Buffer
	b.WriteString(`{"ID":`)
	idJSON, _ := json.Marshal(catID)
	b.Write(idJSON)
	b.WriteString(`,"Color":`)
	colJSON, _ := json.Marshal(color)
	b.Write(colJSON)
	glow := glowColor
	if glow == "" {
		glow = color
	}
	b.WriteString(`,"GlowColor":`)
	glowJSON, _ := json.Marshal(glow)
	b.Write(glowJSON)
	b.WriteByte('}')
	def = b.Bytes()

	arr := body[arrStart:arrEnd]
	if hasCategoryWithID(arr, catID) {
		return replaceCategoryDef(body, arrStart, arrEnd, def)
	}

	insertAt := arrStart + 1
	out := append([]byte(nil), body[:insertAt]...)
	out = append(out, def...)
	if len(bytes.TrimSpace(arr[1:])) > 0 {
		out = append(out, ',')
		out = append(out, body[insertAt:]...)
	} else {
		out = append(out, body[insertAt:]...)
	}
	log.Printf("[title_color] inserted category %q Color=%q GlowColor=%q (%d -> %d bytes)",
		catID, color, glow, len(body), len(out))
	return out, true
}

func playerTitleConfigObject(body []byte) (start, end int, ok bool) {
	key := []byte(`"PlayerTitleConfig"`)
	at := bytes.Index(body, key)
	if at < 0 {
		return 0, 0, false
	}
	i := at + len(key)
	for i < len(body) && isJSONSpace(body[i]) {
		i++
	}
	if i >= len(body) || body[i] != ':' {
		return 0, 0, false
	}
	i++
	for i < len(body) && isJSONSpace(body[i]) {
		i++
	}
	if i >= len(body) || body[i] != '{' {
		return 0, 0, false
	}
	start = i
	closeAt := scanObjectEnd(body, start)
	if closeAt < 0 {
		return 0, 0, false
	}
	return start, closeAt + 1, true
}

func categoriesArrayBounds(body []byte, ptcStart, ptcEnd int) (arrStart, arrEnd int, ok bool) {
	ptc := body[ptcStart:ptcEnd]
	key := []byte(`"Categories"`)
	k := bytes.Index(ptc, key)
	if k < 0 {
		return 0, 0, false
	}
	i := k + len(key)
	for i < len(ptc) && isJSONSpace(ptc[i]) {
		i++
	}
	if i >= len(ptc) || ptc[i] != ':' {
		return 0, 0, false
	}
	i++
	for i < len(ptc) && isJSONSpace(ptc[i]) {
		i++
	}
	if i >= len(ptc) || ptc[i] != '[' {
		return 0, 0, false
	}
	arrStart = ptcStart + i
	closeRel := scanArrayEnd(body, arrStart)
	if closeRel < 0 {
		return 0, 0, false
	}
	return arrStart, closeRel + 1, true
}

func hasCategoryWithID(arr []byte, catID string) bool {
	pat := []byte(`"ID":"` + catID + `"`)
	return bytes.Index(arr, pat) >= 0
}

func replaceCategoryDef(body []byte, arrStart, arrEnd int, def []byte) ([]byte, bool) {
	arr := body[arrStart:arrEnd]
	depth := 0
	inStr := false
	esc := false
	for i := 1; i < len(arr); i++ {
		c := arr[i]
		if inStr {
			if esc {
				esc = false
			} else if c == '\\' {
				esc = true
			} else if c == '"' {
				inStr = false
			}
			continue
		}
		switch c {
		case '"':
			inStr = true
		case '{':
			if depth == 0 {
				objStart := i
				end := scanObjectEnd(arr, objStart)
				if end < 0 {
					return body, false
				}
				obj := arr[objStart : end+1]
				if sameID(obj, def) {
					if bytes.Equal(obj, def) {
						return body, false
					}
					out := append([]byte(nil), body[:arrStart+objStart]...)
					out = append(out, def...)
					out = append(out, body[arrStart+end+1:]...)
					log.Printf("[title_color] replaced existing category def (%d -> %d bytes)", len(body), len(out))
					return out, true
				}
				i = end
			}
		}
	}
	return body, false
}

func sameID(objA, objB []byte) bool {
	a := extractFieldValue(objA, "ID")
	return a != "" && a == extractFieldValue(objB, "ID")
}

func extractFieldValue(flat []byte, field string) string {
	key := []byte(`"` + field + `":"`)
	k := bytes.Index(flat, key)
	if k < 0 {
		return ""
	}
	valStart := k + len(key)
	j := jsonStringEnd(flat, valStart)
	if j < 0 {
		return ""
	}
	return string(flat[valStart:j])
}

func patchTitleColors(body []byte, c SpoofConfig) ([]byte, bool) {
	swaps := c.titleSwaps()
	if len(swaps) == 0 {
		return body, false
	}
	out := body
	changed := false
	for _, s := range swaps {
		tc := s.TitleColor
		if tc.Color == "" {
			continue
		}
        if (!tc.valid()) {
			log.Printf("[title_color] invalid hex color=%q glow=%q — skipped", tc.Color, tc.GlowColor)
			continue
		}

		glow := tc.GlowColor
		if glow == "" {
			glow = tc.Color
		}
		if strings.EqualFold(tc.Color, "FFFFFF") && strings.EqualFold(glow, "FFFFFF") {
			log.Printf("[title_color] skipping all-white Color/Glow for equip (invisible on most banners)")
			continue
		}
		equip := s.equipTitleID()
		if equip == "" {
			continue
		}
		catID := titleCustomCategoryID(equip, s.wantText())
		next, did := upsertTitleCategoryRaw(out, catID, tc.Color, tc.GlowColor)
		if did {
			out = next
			changed = true
		}

		if next2, did2 := replaceEquipCategory(out, equip, catID); did2 {
			out = next2
			changed = true
			log.Printf("[title_color] equip %q Category -> %q", equip, catID)
		}
	}
	if !changed {
		return body, false
	}
	return out, true
}
