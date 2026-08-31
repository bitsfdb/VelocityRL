package main

import (
	"log"
	"net/http"
	"strings"
	"sync/atomic"
	"time"
)

const (
	psyBuildID      = "-204179477"
	psyEnvironment  = "Prod"
	defaultRLUserUA = "RL Win/260811.1257.524913 gzip (x86_64-pc-win32) curl-7.67.0 Schannel"
)

var psyWSExactHeaderNames = map[string]string{
	"psytoken":       "PsyToken",
	"psysessionid":   "PsySessionID",
	"psybuildid":     "PsyBuildID",
	"psyenvironment": "PsyEnvironment",
	"user-agent":     "User-Agent",
	"origin":         "Origin",
}

type authWSCreds struct {
	Token   string
	Session string
	At      time.Time
}

var lastAuthWS atomic.Value

func rememberAuthPlayerWS(body []byte) {
	if len(body) == 0 {
		return
	}
	token, ok1 := extractJSONStringField(body, "PsyToken")
	session, ok2 := extractJSONStringField(body, "SessionID")
	if !ok1 || !ok2 || token == "" || session == "" {
		return
	}
	lastAuthWS.Store(&authWSCreds{Token: token, Session: session, At: time.Now()})
	log.Printf("[auth_ws] cached PsyToken/session for upstream WS (session=%q…)", trunc(session, 8))
}

func setExactHeader(h http.Header, exactKey, val string) {
	for k := range h {
		if strings.EqualFold(k, exactKey) {
			delete(h, k)
		}
	}
	h[exactKey] = []string{val}
}

func headerExactGet(h http.Header, exactKey string) string {
	if v := h.Get(exactKey); v != "" {
		return v
	}
	for k, vv := range h {
		if strings.EqualFold(k, exactKey) && len(vv) > 0 {
			return vv[0]
		}
	}
	return ""
}

func injectCachedAuthWSHeaders(hdr http.Header) {
	if headerExactGet(hdr, "PsyToken") != "" {
		return
	}
	v := lastAuthWS.Load()
	if v == nil {
		return
	}
	c, ok := v.(*authWSCreds)
	if !ok || c.Token == "" || c.Session == "" {
		return
	}
	if time.Since(c.At) > 3*time.Minute {
		return
	}
	setExactHeader(hdr, "PsyToken", c.Token)
	setExactHeader(hdr, "PsySessionID", c.Session)
	if headerExactGet(hdr, "PsyBuildID") == "" {
		setExactHeader(hdr, "PsyBuildID", psyBuildID)
	}
	if headerExactGet(hdr, "PsyEnvironment") == "" {
		setExactHeader(hdr, "PsyEnvironment", psyEnvironment)
	}
	log.Printf("[auth_ws] injected cached PsyToken/PsySessionID on upstream WS dial")
}

func buildUpstreamWSHeaders(client http.Header) http.Header {
	hdr := filterWSHeaders(client)
	normalizePsyWSHeaderCase(hdr)
	injectCachedAuthWSHeaders(hdr)
	o := headerExactGet(hdr, "Origin")
	if o == "" || strings.Contains(o, "127.0.0.1") || strings.Contains(o, "localhost") {
		setExactHeader(hdr, "Origin", "https://"+realWSHost)
	}
	if headerExactGet(hdr, "User-Agent") == "" {
		setExactHeader(hdr, "User-Agent", defaultRLUserUA)
	}
	if headerExactGet(hdr, "PsyBuildID") == "" {
		setExactHeader(hdr, "PsyBuildID", psyBuildID)
	}
	if headerExactGet(hdr, "PsyEnvironment") == "" {
		setExactHeader(hdr, "PsyEnvironment", psyEnvironment)
	}
	return hdr
}

func normalizePsyWSHeaderCase(h http.Header) {
	for k, vv := range h {
		lk := strings.ToLower(k)
		exact, ok := psyWSExactHeaderNames[lk]
		if !ok || k == exact || len(vv) == 0 {
			continue
		}
		delete(h, k)
		h[exact] = append([]string(nil), vv...)
	}
}

func logWSUpgradeAuth(logTag string, sess int64, client, upstream http.Header) {
	log.Printf("%s sess=%d upgrade auth client_token=%v client_session=%v upstream_token=%v upstream_session=%v",
		logTag, sess,
		headerExactGet(client, "PsyToken") != "",
		headerExactGet(client, "PsySessionID") != "",
		headerExactGet(upstream, "PsyToken") != "",
		headerExactGet(upstream, "PsySessionID") != "",
	)
}

func trunc(s string, n int) string {
	if len(s) <= n {
		return s
	}
	return s[:n]
}
