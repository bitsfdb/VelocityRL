package main

import (
	"bytes"
	"encoding/json"
	"log"
	"os"
	"strings"
	"sync"
)

var playerIDPersistMu sync.Mutex

func syncLearnedPlayerIDFromCfg(c SpoofConfig) {
	pid := strings.TrimSpace(c.NameSpoof.PlayerID)
	if pid == "" || isPlaceholderPlayerID(pid) {
		return
	}
	learnedOwnPlayerID.Store(pid)
}

func persistLearnedPlayerID(id string) {
	id = strings.TrimSpace(id)
	if id == "" {
		return
	}
	playerIDPersistMu.Lock()
	defer playerIDPersistMu.Unlock()

	b, err := os.ReadFile(cfgPath)
	if err != nil {
		return
	}
	b = bytes.TrimPrefix(b, []byte{0xEF, 0xBB, 0xBF})
	var root map[string]json.RawMessage
	if err := json.Unmarshal(b, &root); err != nil {
		return
	}
	ns := map[string]interface{}{}
	if raw, ok := root["name_spoof"]; ok && len(raw) > 0 {
		_ = json.Unmarshal(raw, &ns)
	}
	if existing, _ := ns["player_id"].(string); strings.TrimSpace(existing) != "" && !isPlaceholderPlayerID(existing) {
		return
	}
	ns["player_id"] = id
	nsBytes, err := json.Marshal(ns)
	if err != nil {
		return
	}
	root["name_spoof"] = nsBytes
	out, err := json.MarshalIndent(root, "", "    ")
	if err != nil {
		return
	}
	if err := os.WriteFile(cfgPath, out, 0o644); err != nil {
		log.Printf("[name_spoof] auto-save player_id failed: %v", err)
		return
	}
	log.Printf("[name_spoof] auto-saved player_id=%q to %s", id, cfgPath)
}
