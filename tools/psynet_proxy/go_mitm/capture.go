package main

import (
	"encoding/json"
	"net/http"
	"strings"
	"sync"
	"sync/atomic"
	"time"
)

// Minimal no-op capture types so HTTP/WS handlers compile without the research
// traffic viewer or disk dump session (removed for 2.0 production).

type capKey struct{}

type Exchange struct {
	ID             int64             `json:"id"`
	Time           time.Time         `json:"time"`
	Method         string            `json:"method"`
	Host           string            `json:"host"`
	Path           string            `json:"path"`
	Query          string            `json:"query"`
	ReqHeaders     map[string]string `json:"req_headers"`
	ReqBody        json.RawMessage   `json:"req_body"`
	Status         string            `json:"status"`
	StatusCode     int               `json:"status_code"`
	RespHeaders    map[string]string `json:"resp_headers"`
	RespBody       json.RawMessage   `json:"resp_body"`
	RespBodyOrigin json.RawMessage   `json:"resp_body_origin,omitempty"`
	RespBytes      int               `json:"resp_bytes"`
	Patched        bool              `json:"patched"`
	Passthrough    bool              `json:"passthrough,omitempty"`
	Error          string            `json:"error,omitempty"`
	skipDisk       bool
}

type captureStore struct {
	mu  sync.RWMutex
	dir string
	seq atomic.Int64
}

var captures = &captureStore{dir: "captures"}

func (s *captureStore) start() {}

func (s *captureStore) publish(_ *Exchange) {}

func headerMap(h http.Header) map[string]string {
	out := make(map[string]string, len(h))
	for k, vv := range h {
		if len(vv) > 0 {
			out[k] = vv[0]
		}
	}
	return out
}

func asRawJSON(b []byte) json.RawMessage {
	b = bytesTrimSpace(b)
	if len(b) == 0 {
		return json.RawMessage("null")
	}
	if json.Valid(b) {
		return json.RawMessage(append([]byte(nil), b...))
	}
	enc, err := json.Marshal(string(b))
	if err != nil {
		return json.RawMessage("null")
	}
	return json.RawMessage(enc)
}

func bytesTrimSpace(b []byte) []byte {
	return []byte(strings.TrimSpace(string(b)))
}

func publishStaleExchange(r *http.Request, ent staleEntry) {
	if r == nil {
		return
	}
	captures.publish(&Exchange{
		Method:      r.Method,
		Host:        r.Host,
		Path:        r.URL.Path,
		Query:       r.URL.RawQuery,
		ReqHeaders:  headerMap(r.Header),
		Status:      http.StatusText(ent.status),
		StatusCode:  ent.status,
		RespHeaders: headerMap(ent.hdr),
		RespBody:    asRawJSON(ent.body),
		RespBytes:   len(ent.body),
		Passthrough: true,
	})
}
