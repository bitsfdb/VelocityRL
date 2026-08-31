package main

import (
	"bytes"
	"log"
	"strings"
	"sync"
)

const wsRouteMaxPerSession = 1024

type wsRouteTable struct {
	mu    sync.Mutex
	svc   map[string]string
	order []string
}

var wsRoutes sync.Map

func wsRouteTableFor(sess int64) *wsRouteTable {
	if v, ok := wsRoutes.Load(sess); ok {
		return v.(*wsRouteTable)
	}
	v, _ := wsRoutes.LoadOrStore(sess, &wsRouteTable{svc: map[string]string{}})
	return v.(*wsRouteTable)
}

func rememberWSRoute(sess int64, frame []byte) {
	headers, _, ok := splitWSFrame(frame)
	if !ok {
		return
	}
	reqID := wsHeaderValue(headers, "PsyRequestID")
	svc := strings.ToLower(strings.TrimSpace(wsHeaderValue(headers, "PsyService")))
	if reqID == "" || svc == "" {
		return
	}
	t := wsRouteTableFor(sess)
	t.mu.Lock()
	defer t.mu.Unlock()
	if _, exists := t.svc[reqID]; !exists {
		t.order = append(t.order, reqID)
		for len(t.order) > wsRouteMaxPerSession {
			delete(t.svc, t.order[0])
			t.order = t.order[1:]
		}
	}
	t.svc[reqID] = svc
}

func wsServiceForHeaders(sess int64, headers []byte) string {
	if svc := strings.ToLower(strings.TrimSpace(wsHeaderValue(headers, "PsyService"))); svc != "" {
		return svc
	}
	respID := wsHeaderValue(headers, "PsyResponseID")
	if respID == "" {
		return ""
	}
	t := wsRouteTableFor(sess)
	t.mu.Lock()
	defer t.mu.Unlock()
	return t.svc[respID]
}

func wsFrameService(sess int64, frame []byte) string {
	headers, _, ok := splitWSFrame(frame)
	if !ok {
		return ""
	}
	return wsServiceForHeaders(sess, headers)
}

func forgetWSRoutes(sess int64) {
	wsRoutes.Delete(sess)
}

func splitWSFrame(frame []byte) (headers, body []byte, ok bool) {
	hdrEnd := bytes.Index(frame, []byte("\r\n\r\n"))
	if hdrEnd < 0 {
		return nil, nil, false
	}
	return frame[:hdrEnd], frame[hdrEnd+4:], true
}

func logWSPatch(sess int64, dir, feature, svc string, before, after int) {
	shown := svc
	if shown == "" {
		shown = "unknown"
	}
	log.Printf("[ws_patch] sess=%d %s feature=%s svc=%s %d -> %d bytes",
		sess, dir, feature, shown, before, after)
	if isLoadoutSensitiveService(svc) {
		log.Printf("[loadout_guard] BUG: modified loadout-sensitive service svc=%s feature=%s sess=%d — this corrupts the saved loadout, please report",
			svc, feature, sess)
	}
}
