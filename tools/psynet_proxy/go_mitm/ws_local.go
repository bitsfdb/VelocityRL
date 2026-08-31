package main

import (
	"io"
	"log"
	"net/http"
	"net/url"
	"strings"
	"sync"

	"github.com/gorilla/websocket"
)

const realWSHost = "ws.rlpp.psynet.gg"

func handleRewriteWebSocket(w http.ResponseWriter, r *http.Request) {
	c := getCfg()
	if !c.nameSpoofRewriteWSActive() {
		http.Error(w, "ws rewrite off (enable broker or WS spoofs)", http.StatusBadGateway)
		return
	}

	path := r.URL.RequestURI()
	if path == "" || path == "/" {
		if u, err := url.Parse(c.wsLocalURL()); err == nil && u.Path != "" {
			path = u.RequestURI()
		} else {
			path = "/ws/gc2"
		}
	}

	if !strings.HasPrefix(path, "/") {
		path = "/" + path
	}
	upstreamURL := "wss://" + realWSHost + path
	proxyWebSocket(w, r, upstreamURL, "[ws_rewrite]")
}

func proxyWebSocket(w http.ResponseWriter, r *http.Request, upstreamURL, logTag string) {
	sess := wsSessionSeq.Add(1)
	log.Printf("%s upgrade sess=%d %s %s from %s (%s) -> %s",
		logTag, sess, r.Method, r.URL.RequestURI(), r.RemoteAddr, peerProcess(r.RemoteAddr), upstreamURL)

	clientConn, err := wsUpgrader.Upgrade(w, r, nil)
	if err != nil {
		log.Printf("%s client upgrade failed sess=%d: %v", logTag, sess, err)
		return
	}
	defer clientConn.Close()

	hdr := buildUpstreamWSHeaders(r.Header)
	logWSUpgradeAuth(logTag, sess, r.Header, hdr)

	var keys []string
	for k := range hdr {
		keys = append(keys, k)
	}
	log.Printf("%s sess=%d upstream header keys=%v", logTag, sess, keys)
	upConn, resp, err := wsDialer.Dial(upstreamURL, hdr)
	if err != nil {
		log.Printf("%s upstream dial failed sess=%d url=%s: %v", logTag, sess, upstreamURL, err)
		if resp != nil {
			log.Printf("%s upstream status=%s", logTag, resp.Status)
			io.Copy(io.Discard, resp.Body)
			resp.Body.Close()
		}
		_ = clientConn.WriteMessage(websocket.CloseMessage,
			websocket.FormatCloseMessage(websocket.CloseTryAgainLater, "upstream dial failed"))
		return
	}
	defer upConn.Close()
	log.Printf("%s connected sess=%d upstream=%s", logTag, sess, upstreamURL)

	displayName := getCfg().nameSpoofDisplay()
	var wg sync.WaitGroup
	wg.Add(2)
	go func() {
		defer wg.Done()
		pumpWS(sess, "c2s", clientConn, upConn, displayName)
	}()
	go func() {
		defer wg.Done()
		pumpWS(sess, "s2c", upConn, clientConn, displayName)
	}()
	wg.Wait()
	forgetWSRoutes(sess)
	log.Printf("%s session closed sess=%d", logTag, sess)
}
