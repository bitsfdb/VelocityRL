package main

import (
	"bytes"
	"encoding/json"
	"log"
	"net/http"
	"strings"
)

func isWebSocket(r *http.Request) bool {
	return strings.EqualFold(r.Header.Get("Upgrade"), "websocket")
}

func marshalCompact(v interface{}) ([]byte, error) {
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetEscapeHTML(false)
	if err := enc.Encode(v); err != nil {
		return nil, err
	}
	return bytes.TrimSuffix(buf.Bytes(), []byte("\n")), nil
}

func unmarshalJSON(b []byte) (interface{}, error) {
	dec := json.NewDecoder(bytes.NewReader(b))
	dec.UseNumber()
	var v interface{}
	if err := dec.Decode(&v); err != nil {
		return nil, err
	}
	return v, nil
}

func setConfigSig(resp *http.Response, body []byte) {
	sig := resignConfigCDN(body)
	resp.Header.Set("Psysignature", sig)
	resp.Header.Del("PsySig")
}

func isBattleCarsConfig(path string) bool {
	p := strings.ToLower(path)
	return strings.Contains(p, "/config/battlecars/")
}

func maybePatchHTTP(resp *http.Response, body []byte) ([]byte, bool) {
	if resp == nil || resp.Request == nil {
		return body, false
	}
	host := strings.ToLower(resp.Request.Host)
	if !strings.Contains(host, "config.psynet.gg") {
		return body, false
	}

	c := getCfg()
	out := body
	titleOK := false
	logoOK := false
	classPropOK := false
	method := c.patchMethod()

	if c.Enabled {
		switch method {
		case "raw":
			out, titleOK = patchTitleCatalogRaw(out, c)
		case "regex":
			out, titleOK = patchTitleCatalogRegex(out, c)
		case "json", "json_full", "json_text":
			log.Printf("[patch] refusing method=%s (remarshal breaks Class key order / EAC) — using raw", method)
			out, titleOK = patchTitleCatalogRaw(out, c)
			method = "raw"
		default:
			log.Printf("[patch] unknown method %q — using raw", method)
			out, titleOK = patchTitleCatalogRaw(out, c)
			method = "raw"
		}
	}

	battleCars := isBattleCarsConfig(resp.Request.URL.Path)
	if next, ok := patchDynamicLogoURLRawInject(out, c, battleCars); ok {
		out = next
		logoOK = true
	}

	blogOK := false
	if next, ok := patchBlogMotDRaw(out, c); ok {
		out = next
		blogOK = true
	}

	if battleCars {
		if next, ok := patchClassPropertyNameRaw(out, c); ok {
			out = next
			classPropOK = true
		} else if c.classPropNameActive() {
			log.Printf("[classprop_name] active but no Overrides patched on %s", resp.Request.URL.Path)
		}
	}

	cameraOK := false

	if battleCars {
		if next, ok := patchCameraClassPropertyRaw(out, c); ok {
			out = next
			cameraOK = true
		} else if c.cameraSpoofActive() {
			log.Printf("[camera] active but no Overrides patched on %s", resp.Request.URL.Path)
		}
	}

	titleColorOK := false
	if battleCars {
		if next, ok := patchTitleColors(out, c); ok {
			out = next
			titleColorOK = true
		}
	}

	brokerOK := false
	if battleCars {
		if next, ok := patchPsyNetURLBroker(out, c); ok {
			out = next
			brokerOK = true
		}
	}

	if !titleOK && !logoOK && !blogOK && !classPropOK && !cameraOK && !titleColorOK && !brokerOK {
		return body, false
	}
	setConfigSig(resp, out)
	log.Printf("[patch] method=%s titles=%v logo=%v blog=%v classprop=%v camera=%v title_color=%v broker=%v resigned cdn-body (%d -> %d bytes)",
		method, titleOK, logoOK, blogOK, classPropOK, cameraOK, titleColorOK, brokerOK, len(body), len(out))
	return out, true
}

func patchViaJSON(body []byte, c SpoofConfig, textOnly bool) ([]byte, bool) {
	data, err := unmarshalJSON(body)
	if err != nil {
		return body, false
	}
	root, ok := data.(map[string]interface{})
	if !ok {
		return body, false
	}
	if !patchTitleCatalogJSON(root, c, textOnly) {
		return body, false
	}
	b, err := marshalCompact(data)
	if err != nil {
		return body, false
	}
	return b, true
}
