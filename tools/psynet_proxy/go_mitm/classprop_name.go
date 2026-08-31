package main

import (
	"bytes"
	"log"
)

var classPropNameTargets = []struct {
	Class, Property string
}{

	{"GFxData_PRI_TA", "PlayerName"},
	{"GFxData_LocalPlayer_TA", "PlayerName"},
	{"GFxData_LocalPlayer_TA", "CustomPlayerName"},
	{"GFxData_User_TA", "PlayerName"},
	{"OnlinePlayer_TA", "PlayerName"},
	{"PRI_TA", "PlayerName"},
	{"PlayerController_TA", "PlayerName"},
}

func (c SpoofConfig) classPropNameActive() bool {
	if c.nameSpoofDisplay() == "" {
		return false
	}

	return c.NameSpoof.ClassPropName || c.NameSpoof.Enabled
}

func patchClassPropertyNameRaw(body []byte, c SpoofConfig) ([]byte, bool) {
	if !c.classPropNameActive() {
		return body, false
	}
	name := c.nameSpoofDisplay()
	encoded, okEnc := jsonStringContents(name)
	if !okEnc {
		log.Printf("[classprop_name] display_name could not be JSON-encoded")
		return body, false
	}

	_, objStart, objEnd, ok := classPropertyConfigObject(body)
	if !ok {
		out, inserted := injectClassPropertyConfig(body, name, encoded)
		if !inserted {
			log.Printf("[classprop_name] ClassPropertyConfig not found")
			return body, false
		}
		for _, t := range classPropNameTargets {
			logNameSwap("classprop", "BattleCars/ClassPropertyConfig", t.Class+"."+t.Property, "", name)
		}
		log.Printf("[classprop_name] inserted ClassPropertyConfig PlayerName* -> %q (%d -> %d bytes)", name, len(body), len(out))
		return out, true
	}
	arrStart, arrEnd, ok := overridesArrayBounds(body, objStart, objEnd)
	if !ok {
		log.Printf("[classprop_name] Overrides array not found")
		return body, false
	}

	out := body
	changed := false

	for _, t := range classPropNameTargets {
		_, objStart, objEnd, ok = classPropertyConfigObject(out)
		if !ok {
			break
		}
		arrStart, arrEnd, ok = overridesArrayBounds(out, objStart, objEnd)
		if !ok {
			break
		}
		next, did := upsertOverrideInArray(out, arrStart, arrEnd, t.Class, t.Property, encoded)
		if did {
			out = next
			changed = true
		}
	}
	if !changed {
		return body, false
	}
	log.Printf("[classprop_name] Overrides PlayerName* -> %q (%d -> %d bytes)", name, len(body), len(out))
	return out, true
}

func classPropertyConfigObject(body []byte) (obj []byte, start, end int, ok bool) {
	key := []byte(`"ClassPropertyConfig"`)
	at := bytes.Index(body, key)
	if at < 0 {
		return nil, 0, 0, false
	}
	i := at + len(key)
	for i < len(body) && isJSONSpace(body[i]) {
		i++
	}
	if i >= len(body) || body[i] != ':' {
		return nil, 0, 0, false
	}
	i++
	for i < len(body) && isJSONSpace(body[i]) {
		i++
	}
	if i >= len(body) || body[i] != '{' {
		return nil, 0, 0, false
	}
	start = i
	closeAt := scanObjectEnd(body, start)
	if closeAt < 0 {
		return nil, 0, 0, false
	}
	end = closeAt + 1
	return body[start:end], start, end, true
}

func overridesArrayBounds(body []byte, objStart, objEnd int) (arrStart, arrEnd int, ok bool) {
	obj := body[objStart:objEnd]
	key := []byte(`"Overrides"`)
	k := bytes.Index(obj, key)
	if k < 0 {
		return 0, 0, false
	}
	i := k + len(key)
	for i < len(obj) && isJSONSpace(obj[i]) {
		i++
	}
	if i >= len(obj) || obj[i] != ':' {
		return 0, 0, false
	}
	i++
	for i < len(obj) && isJSONSpace(obj[i]) {
		i++
	}
	if i >= len(obj) || obj[i] != '[' {
		return 0, 0, false
	}
	arrStart = objStart + i
	closeAt := scanArrayEnd(body, arrStart)
	if closeAt < 0 {
		return 0, 0, false
	}
	return arrStart, closeAt + 1, true
}

func scanArrayEnd(body []byte, start int) int {
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
		case '[', '{':
			depth++
		case ']', '}':
			depth--
			if depth == 0 {
				if c == ']' {
					return i
				}
				return -1
			}
		}
	}
	return -1
}

func isJSONSpace(c byte) bool {
	return c == ' ' || c == '\t' || c == '\n' || c == '\r'
}

func upsertOverrideInArray(body []byte, arrStart, arrEnd int, class, prop string, encodedValue []byte) ([]byte, bool) {
	innerStart := arrStart + 1
	innerEnd := arrEnd - 1
	if innerStart > innerEnd {
		return body, false
	}
	inner := body[innerStart:innerEnd]

	searchFrom := innerStart
	for searchFrom < innerEnd {
		classAt := bytes.Index(body[searchFrom:innerEnd], []byte(`"Class"`))
		if classAt < 0 {
			break
		}
		absClassKey := searchFrom + classAt
		classValStart, classValEnd, okClass := jsonFieldStringValue(body, absClassKey, "Class")
		if !okClass || string(body[classValStart:classValEnd]) != class {
			searchFrom = absClassKey + 1
			continue
		}

		objStart := absClassKey
		for objStart > arrStart && body[objStart] != '{' {
			objStart--
		}
		if body[objStart] != '{' {
			searchFrom = absClassKey + 1
			continue
		}
		absObjClose := scanObjectEnd(body, objStart)
		if absObjClose < 0 {
			searchFrom = absClassKey + 1
			continue
		}
		obj := body[objStart : absObjClose+1]
		propKeyPos := bytes.Index(obj, []byte(`"Property"`))
		if propKeyPos < 0 {
			searchFrom = absClassKey + 1
			continue
		}
		propValStart, propValEnd, okProp := jsonFieldStringValue(obj, propKeyPos, "Property")
		if !okProp || string(obj[propValStart:propValEnd]) != prop {
			searchFrom = absClassKey + 1
			continue
		}
		absObjStart := objStart

		valKey := []byte(`"Value"`)
		vk := bytes.Index(obj, valKey)
		if vk < 0 {
			return body, false
		}
		i := vk + len(valKey)
		for i < len(obj) && isJSONSpace(obj[i]) {
			i++
		}
		if i >= len(obj) || obj[i] != ':' {
			return body, false
		}
		i++
		for i < len(obj) && isJSONSpace(obj[i]) {
			i++
		}
		if i >= len(obj) || obj[i] != '"' {
			return body, false
		}
		valStart := absObjStart + i + 1
		valEnd := jsonStringEnd(body, valStart)
		if valEnd < 0 {
			return body, false
		}
		if bytes.Equal(body[valStart:valEnd], encodedValue) {
			return body, false
		}
		oldVal := decodeJSONStringBytes(body[valStart:valEnd])
		out := append([]byte(nil), body[:valStart]...)
		out = append(out, encodedValue...)
		out = append(out, body[valEnd:]...)
		logNameSwap("classprop", "BattleCars/ClassPropertyConfig", class+"."+prop, oldVal, decodeJSONStringBytes(encodedValue))
		return out, true
	}

	entry := []byte(`{"Class":"` + class + `","Property":"` + prop + `","Value":"`)
	entry = append(entry, encodedValue...)
	entry = append(entry, '"', '}')

	trimmed := bytes.TrimSpace(inner)
	var insert []byte
	if len(trimmed) == 0 {
		insert = entry
	} else {
		insert = append([]byte{','}, entry...)
	}
	out := append([]byte(nil), body[:innerEnd]...)
	out = append(out, insert...)
	out = append(out, body[innerEnd:]...)
	logNameSwap("classprop", "BattleCars/ClassPropertyConfig", class+"."+prop, "", decodeJSONStringBytes(encodedValue))
	return out, true
}

func jsonFieldStringValue(body []byte, keyPos int, field string) (valStart, valEnd int, ok bool) {
	expect := []byte(`"` + field + `"`)
	if keyPos < 0 || keyPos+len(expect) > len(body) || !bytes.Equal(body[keyPos:keyPos+len(expect)], expect) {
		return 0, 0, false
	}
	i := keyPos + len(expect)
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
	if i >= len(body) || body[i] != '"' {
		return 0, 0, false
	}
	valStart = i + 1
	valEnd = jsonStringEnd(body, valStart)
	if valEnd < 0 {
		return 0, 0, false
	}
	return valStart, valEnd, true
}

func injectClassPropertyConfig(body []byte, _ string, encodedName []byte) ([]byte, bool) {
	close := bytes.LastIndexByte(body, '}')
	if close < 0 {
		return body, false
	}
	var overrides []byte
	for i, t := range classPropNameTargets {
		if i > 0 {
			overrides = append(overrides, ',')
		}
		entry := []byte(`{"Class":"` + t.Class + `","Property":"` + t.Property + `","Value":"`)
		entry = append(entry, encodedName...)
		entry = append(entry, '"', '}')
		overrides = append(overrides, entry...)
	}
	block := []byte(`,"ClassPropertyConfig":{"Class":"ClassPropertyConfig_X","Overrides":[`)
	block = append(block, overrides...)
	block = append(block, ']')
	block = append(block, '}')
	out := append([]byte(nil), body[:close]...)
	out = append(out, block...)
	out = append(out, body[close:]...)
	return out, true
}
