package main

import (
	"net/http"
	"sync"
	"time"
)

type staleEntry struct {
	status int
	hdr    http.Header
	body   []byte
	at     time.Time
}

var (
	staleMu sync.Mutex
	stale   = map[string]staleEntry{}
)

func staleKey(r *http.Request) string {
	if r == nil || r.URL == nil {
		return ""
	}
	return r.Host + " " + r.Method + " " + r.URL.Path
}

func staleSave(r *http.Request, status int, hdr http.Header, body []byte) {
	if r == nil || r.Method != http.MethodGet || status != 200 || len(body) == 0 {
		return
	}
	key := staleKey(r)
	cp := hdr.Clone()
	staleMu.Lock()
	stale[key] = staleEntry{status: status, hdr: cp, body: append([]byte(nil), body...), at: time.Now()}
	staleMu.Unlock()
}

func staleServe(w http.ResponseWriter, r *http.Request) bool {
	ent, ok := staleLookup(r)
	if !ok {
		return false
	}
	staleWrite(w, ent)
	return true
}

func staleLookup(r *http.Request) (staleEntry, bool) {
	key := staleKey(r)
	staleMu.Lock()
	ent, ok := stale[key]
	staleMu.Unlock()
	return ent, ok
}

func staleWrite(w http.ResponseWriter, ent staleEntry) {
	for k, vs := range ent.hdr {
		for _, v := range vs {
			w.Header().Add(k, v)
		}
	}
	w.Header().Set("X-VelocityRL-Cache", "last-good")
	w.WriteHeader(ent.status)
	_, _ = w.Write(ent.body)
}
