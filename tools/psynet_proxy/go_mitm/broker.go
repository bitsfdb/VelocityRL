package main

import (
	"bytes"
	"crypto/tls"
	"fmt"
	"io"
	"log"
	"net/http"
	"strings"
	"sync"
	"time"
)

const (
	brokerHost = "127.0.0.1"
	brokerPort = "27505"
)

func brokerListenAddr() string {
	return brokerHost + ":" + brokerPort
}

func brokerBaseURL() string {
	return "http://" + brokerListenAddr()
}

func brokerRPCURL() string {
	return brokerBaseURL() + "/rpc"
}

func brokerServicesURL() string {
	return brokerBaseURL() + "/Services"
}

var (
	brokerOnce sync.Once
	brokerHTTP = &http.Client{
		Timeout: 45 * time.Second,
		Transport: &http.Transport{
			TLSClientConfig: &tls.Config{InsecureSkipVerify: true, MinVersion: tls.VersionTLS12},
			DialContext:     dialReal,

			ForceAttemptHTTP2: false,
		},
	}
)

func startRPCBroker() {
	brokerOnce.Do(func() {
		mux := http.NewServeMux()
		mux.HandleFunc("/", handleBroker)
		addr := brokerListenAddr()
		go func() {
			log.Printf("[broker] listening on http://%s (PsyNetUrl RPC + local WS forward)", addr)
			if err := http.ListenAndServe(addr, mux); err != nil {
				log.Printf("[broker] listen failed: %v", err)
			}
		}()
	})
}

func handleBroker(w http.ResponseWriter, r *http.Request) {

	if isWebSocket(r) || strings.HasPrefix(strings.ToLower(r.URL.Path), "/ws") {
		if isWebSocket(r) {
			handleRewriteWebSocket(w, r)
			return
		}
	}

	path := r.URL.Path
	if path == "" {
		path = "/"
	}
	upURL := "https://api.rlpp.psynet.gg" + path
	if r.URL.RawQuery != "" {
		upURL += "?" + r.URL.RawQuery
	}

	var reqBody []byte
	if r.Body != nil && r.Method != http.MethodGet && r.Method != http.MethodHead {
		reqBody, _ = io.ReadAll(r.Body)
		_ = r.Body.Close()
	}

	upReq, err := http.NewRequestWithContext(r.Context(), r.Method, upURL, bytes.NewReader(reqBody))
	if err != nil {
		http.Error(w, "broker upstream request", http.StatusBadGateway)
		return
	}
	copyBrokerHeaders(upReq.Header, r.Header)
	upReq.Host = "api.rlpp.psynet.gg"
	upReq.ContentLength = int64(len(reqBody))
	if len(reqBody) > 0 {
		upReq.Header.Set("Content-Length", fmt.Sprintf("%d", len(reqBody)))
	}

	log.Printf("[broker] >>> %s %s (%d bytes)", r.Method, path, len(reqBody))
	resp, err := brokerHTTP.Do(upReq)
	if err != nil {
		log.Printf("[broker] upstream error: %v", err)
		http.Error(w, "broker upstream failed", http.StatusBadGateway)
		return
	}
	defer resp.Body.Close()
	respBody, _ := io.ReadAll(resp.Body)

	patched := false
	originBody := respBody
	if isAuthPlayerPath(path) {
		c := getCfg()
		if c.nameSpoofRewriteWSActive() {
			localV2 := c.wsLocalURL()
			if next, ok := patchAuthPlayerWSURL(respBody, localV2); ok {
				respBody = next
				patched = true
				log.Printf("[broker] AuthPlayer WS URL rewritten -> %s", localV2)
			} else {
				log.Printf("[broker] AuthPlayer WS URL unchanged (fields missing?)")
			}
		}
	}

	outHdr := make(http.Header)
	for k, vv := range resp.Header {
		for _, v := range vv {
			outHdr.Add(k, v)
		}
	}
	if patched {
		resignRPCHeader(outHdr, respBody)
		probeSignature(outHdr, respBody)
	}

	if isAuthPlayerPath(path) {
		if resp.StatusCode == http.StatusOK {
			rememberAuthPlayerWS(respBody)
		}
		publishBrokerExchange(r, reqBody, resp, originBody, respBody, patched, outHdr)
	}

	for k, vv := range outHdr {
		if strings.EqualFold(k, "Content-Length") || strings.EqualFold(k, "Transfer-Encoding") {
			continue
		}
		for _, v := range vv {
			w.Header().Add(k, v)
		}
	}
	w.Header().Set("Content-Length", fmt.Sprintf("%d", len(respBody)))
	w.WriteHeader(resp.StatusCode)
	_, _ = w.Write(respBody)
	log.Printf("[broker] <<< %s %s status=%d (%d bytes)", r.Method, path, resp.StatusCode, len(respBody))
}

func publishBrokerExchange(r *http.Request, reqBody []byte, resp *http.Response, origin, final []byte, patched bool, wireHdr http.Header) {
	respHdr := resp.Header
	if wireHdr != nil {
		respHdr = wireHdr
	}
	ex := &Exchange{
		Method:      r.Method,
		Host:        "127.0.0.1:" + brokerPort + " (broker→api.rlpp)",
		Path:        r.URL.Path,
		Query:       r.URL.RawQuery,
		ReqHeaders:  headerMap(r.Header),
		ReqBody:     asRawJSON(reqBody),
		Status:      resp.Status,
		StatusCode:  resp.StatusCode,
		RespHeaders: headerMap(respHdr),
		RespBody:    asRawJSON(final),
		RespBytes:   len(final),
		Patched:     patched,
	}
	if patched && !bytes.Equal(origin, final) {
		ex.RespBodyOrigin = asRawJSON(origin)
	}
	captures.publish(ex)
}

func copyBrokerHeaders(dst, src http.Header) {
	hop := map[string]bool{
		"connection": true, "keep-alive": true, "proxy-authenticate": true,
		"proxy-authorization": true, "te": true, "trailers": true,
		"transfer-encoding": true, "upgrade": true, "host": true,
		"content-length": true,
	}
	for k, vv := range src {
		if hop[strings.ToLower(k)] {
			continue
		}
		for _, v := range vv {
			dst.Add(k, v)
		}
	}
}

func patchPsyNetURLBroker(body []byte, c SpoofConfig) ([]byte, bool) {
	if !c.psyNetURLBrokerRewriteActive() {
		return body, false
	}
	objStart, objEnd, ok := findNamedObject(body, "PsyNetUrl")
	if !ok {
		log.Printf("[broker] PsyNetUrl object not found")
		return body, false
	}
	obj := append([]byte(nil), body[objStart:objEnd]...)
	changed := false
	if next, did := replaceJSONStringField(obj, "URLv2", brokerRPCURL()); did {
		obj = next
		changed = true
	}
	if next, did := replaceJSONStringField(obj, "URL", brokerServicesURL()); did {
		obj = next
		changed = true
	}
	if !changed {
		log.Printf("[broker] PsyNetUrl URL/URLv2 already set or missing")
		return body, false
	}
	out := append([]byte(nil), body[:objStart]...)
	out = append(out, obj...)
	out = append(out, body[objEnd:]...)
	log.Printf("[broker] PsyNetUrl -> %s / %s", brokerServicesURL(), brokerRPCURL())
	return out, true
}

func findNamedObject(body []byte, name string) (start, end int, ok bool) {
	key := []byte(`"` + name + `"`)
	at := bytes.Index(body, key)
	if at < 0 {
		return 0, 0, false
	}
	i := at + len(key)
	for i < len(body) && isJSONSpace(body[i]) {
		i++
	}
	if i >= len(body) || body[i] != ':' {
		return 0, 0, false
	}
	i++
	for i < len(body) && isJSONSpace(body[i]) {
		i++
	}
	if i >= len(body) || body[i] != '{' {
		return 0, 0, false
	}
	start = i
	closeAt := scanObjectEnd(body, start)
	if closeAt < 0 {
		return 0, 0, false
	}
	return start, closeAt + 1, true
}
