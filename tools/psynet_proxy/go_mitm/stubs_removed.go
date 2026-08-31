package main

// Config stubs for features removed from the shipping proxy binary.
// JSON fields still parse so old configs do not fail; active() always false.

type PingSpoofConfig struct {
	Enabled bool `json:"enabled"`
	Ms      int  `json:"ms"`
}

func (c SpoofConfig) pingSpoofActive() bool { return false }

type InventorySpoofConfig struct {
	Enabled            bool                 `json:"enabled"`
	Items              []InventorySpoofItem `json:"items"`
	JoinAppendSameSlot bool                 `json:"join_append_same_slot,omitempty"`
}

type InventorySpoofItem struct {
	ProductID int    `json:"product_id"`
	PaintID   int    `json:"paint_id"`
	SeriesID  int    `json:"series_id"`
	Slot      string `json:"slot,omitempty"`
	DLC       bool   `json:"dlc,omitempty"`
}

func (c SpoofConfig) inventorySpoofActive() bool { return false }
