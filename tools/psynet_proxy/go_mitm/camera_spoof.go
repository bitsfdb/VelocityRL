package main

import (
	"bytes"
	"fmt"
	"log"
)

type CameraLimit struct {
	Min      float64 `json:"min"`
	Max      float64 `json:"max"`
	Interval float64 `json:"interval"`
}

type CameraSpoofConfig struct {
	Enabled  bool        `json:"enabled"`
	FOV      CameraLimit `json:"fov"`
	Height   CameraLimit `json:"height"`
	Distance CameraLimit `json:"distance"`
}

var (
	defaultCameraFOV      = CameraLimit{Min: 60, Max: 1000, Interval: 1}
	defaultCameraHeight   = CameraLimit{Min: 40, Max: 1000, Interval: 1}
	defaultCameraDistance = CameraLimit{Min: 100, Max: 1000, Interval: 1}
)

func (c SpoofConfig) cameraSpoofActive() bool {
	return c.CameraSpoof.Enabled
}

func (l CameraLimit) resolved(def CameraLimit) CameraLimit {
	out := l
	if out.Max <= 0 && out.Min <= 0 {
		return def
	}
	if out.Interval <= 0 {
		out.Interval = def.Interval
	}
	if out.Max < out.Min {
		out.Max = out.Min
	}
	return out
}

func formatCameraLimit(l CameraLimit) string {
	return fmt.Sprintf("(Min=%.6f,Max=%.6f,interval=%.6f)", l.Min, l.Max, l.Interval)
}

type cameraLimitTarget struct {
	Property string
	resolve  func(CameraSpoofConfig) CameraLimit
}

var cameraClassPropTargets = []cameraLimitTarget{
	{"FOVLimits", func(c CameraSpoofConfig) CameraLimit { return c.FOV.resolved(defaultCameraFOV) }},
	{"HeightLimits", func(c CameraSpoofConfig) CameraLimit { return c.Height.resolved(defaultCameraHeight) }},
	{"DistanceLimits", func(c CameraSpoofConfig) CameraLimit { return c.Distance.resolved(defaultCameraDistance) }},
}

const cameraTAClass = "Camera_TA"

type cameraOverrideEntry struct {
	prop    string
	encoded []byte
	value   string
}

func patchCameraClassPropertyRaw(body []byte, c SpoofConfig) ([]byte, bool) {
	if !c.cameraSpoofActive() {
		return body, false
	}

	entries := make([]cameraOverrideEntry, 0, len(cameraClassPropTargets))
	for _, t := range cameraClassPropTargets {
		val := formatCameraLimit(t.resolve(c.CameraSpoof))
		encoded, okEnc := jsonStringContents(val)
		if !okEnc {
			log.Printf("[camera] %s value could not be JSON-encoded", t.Property)
			return body, false
		}
		entries = append(entries, cameraOverrideEntry{prop: t.Property, encoded: encoded, value: val})
	}

	_, _, _, ok := classPropertyConfigObject(body)
	if !ok {
		out, inserted := injectCameraClassPropertyConfig(body, entries)
		if !inserted {
			log.Printf("[camera] ClassPropertyConfig not found and inject failed")
			return body, false
		}
		for _, e := range entries {
			log.Printf("[camera] inserted Camera_TA.%s -> %s", e.prop, e.value)
		}
		log.Printf("[camera] inserted ClassPropertyConfig Camera_TA limits (%d -> %d bytes)", len(body), len(out))
		return out, true
	}

	out := body
	changed := false
	for _, e := range entries {
		_, objStart, objEnd, ok := classPropertyConfigObject(out)
		if !ok {
			break
		}
		arrStart, arrEnd, ok := overridesArrayBounds(out, objStart, objEnd)
		if !ok {
			log.Printf("[camera] Overrides array not found")
			break
		}
		next, did := upsertOverrideInArray(out, arrStart, arrEnd, cameraTAClass, e.prop, e.encoded)
		if did {
			out = next
			changed = true
		}
	}
	if !changed {
		return body, false
	}
	log.Printf("[camera] Overrides Camera_TA FOV/Height/Distance limits (%d -> %d bytes)", len(body), len(out))
	return out, true
}

func injectCameraClassPropertyConfig(body []byte, entries []cameraOverrideEntry) ([]byte, bool) {
	close := bytes.LastIndexByte(body, '}')
	if close < 0 {
		return body, false
	}
	var overrides []byte
	for i, e := range entries {
		if i > 0 {
			overrides = append(overrides, ',')
		}
		entry := []byte(`{"Class":"` + cameraTAClass + `","Property":"` + e.prop + `","Value":"`)
		entry = append(entry, e.encoded...)
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
