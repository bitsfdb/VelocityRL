package main

import (
	"log"
	"strings"
)

func (c SpoofConfig) equipTitleID() string {
	if c.EquipTitleID != "" {
		return c.EquipTitleID
	}
	return c.SwappedTitleID
}

func (c SpoofConfig) titleSwaps() []SpoofConfig {
	if len(c.Swaps) > 0 {
		out := make([]SpoofConfig, 0, len(c.Swaps))
		for _, s := range c.Swaps {
			one := SpoofConfig{
				Enabled:          c.Enabled,
				EquipTitleID:     s.EquipTitleID,
				DisplayTitleID:   s.DisplayTitleID,
				DisplayAsTitleID: s.DisplayAsTitleID,
				SwappedTitleID:   s.SwappedTitleID,
				CustomText:       s.CustomText,
				Category:         s.Category,
				TitleColor:       s.TitleColor,
				Method:           c.Method,
			}
			if one.equipTitleID() == "" {
				continue
			}
			out = append(out, one)
		}
		return out
	}
	if c.equipTitleID() == "" {
		return nil
	}
	legacy := c
	legacy.Swaps = nil
	return []SpoofConfig{legacy}
}

func (c SpoofConfig) displayTitleID() string {
	if c.DisplayTitleID != "" {
		return c.DisplayTitleID
	}
	return c.DisplayAsTitleID
}

func (c SpoofConfig) patchMethod() string {
	m := strings.ToLower(strings.TrimSpace(c.Method))
	if m == "" {

		return "raw"
	}
	return m
}

func (c SpoofConfig) wantText() string {
	return strings.TrimSpace(c.CustomText)
}

func (c SpoofConfig) wantCategory() string {
	return strings.TrimSpace(c.Category)
}

func patchTitleCatalogJSON(root map[string]interface{}, c SpoofConfig, textOnly bool) bool {
	swaps := c.titleSwaps()
	if len(swaps) == 0 {
		return false
	}
	changed := false
	for _, s := range swaps {
		if patchOneTitleCatalogJSON(root, s, textOnly) {
			changed = true
		}
	}
	return changed
}

func patchOneTitleCatalogJSON(root map[string]interface{}, c SpoofConfig, textOnly bool) bool {
	equip := c.equipTitleID()
	if equip == "" {
		return false
	}

	ptc, ok := root["PlayerTitleConfig"].(map[string]interface{})
	if !ok {
		return false
	}
	titles, ok := ptc["Titles"].([]interface{})
	if !ok {
		return false
	}

	var src, dst map[string]interface{}
	display := c.displayTitleID()
	for _, item := range titles {
		m, ok := item.(map[string]interface{})
		if !ok {
			continue
		}
		id, _ := m["ID"].(string)
		if id == equip {
			dst = m
		}
		if display != "" && id == display {
			src = m
		}
	}
	if dst == nil {
		log.Printf("[patch] catalog json: equip %q not found", equip)
		return false
	}

	text := c.wantText()
	if text == "" && src != nil {
		text, _ = src["Text"].(string)
	}
	if text == "" {
		log.Printf("[patch] catalog json: no custom_text / display Text")
		return false
	}

	changed := false
	if cur, _ := dst["Text"].(string); cur != text {
		dst["Text"] = text
		changed = true
	}

	if !textOnly {
		cat := c.wantCategory()
		if cat == "" && src != nil {
			cat, _ = src["Category"].(string)
		}
		if cat != "" {
			if cur, _ := dst["Category"].(string); cur != cat {
				dst["Category"] = cat
				changed = true
			}
		} else {
			log.Printf("[patch] catalog json: no category — glow will stay default")
		}
	}

	if changed {
		cat, _ := dst["Category"].(string)
		log.Printf("[patch] catalog json: %q Text=%q Category=%q (glow from Categories)", equip, text, cat)
	}
	return changed
}
