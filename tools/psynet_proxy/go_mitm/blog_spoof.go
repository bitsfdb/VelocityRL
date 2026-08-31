package main

import (
	"bytes"
	"log"
	"strings"
)

type BlogSpoofConfig struct {
	Enabled bool   `json:"enabled"`
	MotD    string `json:"motd"`
}

func (c SpoofConfig) blogSpoofActive() bool {
	return c.BlogSpoof.Enabled && strings.TrimSpace(c.BlogSpoof.MotD) != ""
}

func patchBlogMotDRaw(body []byte, c SpoofConfig) ([]byte, bool) {
	if !c.blogSpoofActive() {
		return body, false
	}
	want := strings.TrimSpace(c.BlogSpoof.MotD)
	encoded, okEnc := jsonStringContents(want)
	if !okEnc {
		log.Printf("[blog_spoof] MotD could not be JSON-encoded")
		return body, false
	}

	obj, objStart, _, ok := blogConfigObject(body)
	if !ok {
		out, inserted := injectBlogConfig(body, encoded)
		if !inserted {
			log.Printf("[blog_spoof] BlogConfig not found and inject failed")
			return body, false
		}
		log.Printf("[blog_spoof] inserted BlogConfig.MotD -> %q (%d -> %d bytes)", want, len(body), len(out))
		return out, true
	}

	valStart, ok := motdValueStart(obj, objStart)
	if !ok {
		log.Printf("[blog_spoof] MotD field not found in BlogConfig")
		return body, false
	}
	valEnd := jsonStringEnd(body, valStart)
	if valEnd < 0 {
		return body, false
	}
	if bytes.Equal(body[valStart:valEnd], encoded) {
		return body, false
	}

	out := append([]byte(nil), body[:valStart]...)
	out = append(out, encoded...)
	out = append(out, body[valEnd:]...)
	log.Printf("[blog_spoof] BlogConfig.MotD -> %q (%d -> %d bytes)", want, len(body), len(out))
	return out, true
}

func blogConfigObject(body []byte) (obj []byte, start, end int, ok bool) {
	key := []byte(`"BlogConfig"`)
	at := bytes.Index(body, key)
	if at < 0 {
		return nil, 0, 0, false
	}
	i := at + len(key)
	for i < len(body) && (body[i] == ' ' || body[i] == '\t' || body[i] == '\n' || body[i] == '\r') {
		i++
	}
	if i >= len(body) || body[i] != ':' {
		return nil, 0, 0, false
	}
	i++
	for i < len(body) && (body[i] == ' ' || body[i] == '\t' || body[i] == '\n' || body[i] == '\r') {
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

func motdValueStart(obj []byte, objStart int) (valStart int, ok bool) {
	key := []byte(`"MotD"`)
	k := bytes.Index(obj, key)
	if k < 0 {
		return 0, false
	}
	i := k + len(key)
	for i < len(obj) && (obj[i] == ' ' || obj[i] == '\t' || obj[i] == '\n' || obj[i] == '\r') {
		i++
	}
	if i >= len(obj) || obj[i] != ':' {
		return 0, false
	}
	i++
	for i < len(obj) && (obj[i] == ' ' || obj[i] == '\t' || obj[i] == '\n' || obj[i] == '\r') {
		i++
	}
	if i >= len(obj) || obj[i] != '"' {
		return 0, false
	}
	return objStart + i + 1, true
}

func injectBlogConfig(body []byte, encodedMotD []byte) ([]byte, bool) {
	close := bytes.LastIndexByte(body, '}')
	if close < 0 {
		return body, false
	}
	block := []byte(`,"BlogConfig":{"Class":"BlogConfig_X","MotD":"`)
	block = append(block, encodedMotD...)
	block = append(block, '"', '}')
	out := append([]byte(nil), body[:close]...)
	out = append(out, block...)
	out = append(out, body[close:]...)
	return out, true
}
