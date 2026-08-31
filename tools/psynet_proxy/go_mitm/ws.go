package main

import (
	"bytes"
	"crypto/hmac"
	"crypto/sha256"
	"crypto/tls"
	"encoding/base64"
	"encoding/hex"
	"fmt"
	"log"
	"net/http"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"github.com/gorilla/websocket"
)

var (
	wsUpgrader = websocket.Upgrader{
		CheckOrigin:     func(r *http.Request) bool { return true },
		ReadBufferSize:  64 * 1024,
		WriteBufferSize: 64 * 1024,
	}
	wsDialer = websocket.Dialer{
		Proxy:            http.ProxyFromEnvironment,
		HandshakeTimeout: 12 * time.Second,
		TLSClientConfig: &tls.Config{
			InsecureSkipVerify: true,
			MinVersion:         tls.VersionTLS12,
		},
		NetDialContext:  dialReal,
		ReadBufferSize:  64 * 1024,
		WriteBufferSize: 64 * 1024,
	}
	wsSessionSeq atomic.Int64
	wsTLSRejects atomic.Int64
	wsFirstFrame sync.Map

	wsRecentHello sync.Map
)

func handleWebSocket(w http.ResponseWriter, r *http.Request) {
	c := getCfg()
	if !c.nameSpoofWSActive() {
		http.Error(w, "ws.rlpp not intercepted (name_spoof.websocket off)", http.StatusBadGateway)
		return
	}
	host := strings.ToLower(r.Host)
	if i := strings.IndexByte(host, ':'); i >= 0 {
		host = host[:i]
	}
	if !strings.Contains(host, "ws.rlpp.psynet.gg") {
		http.Error(w, "websocket host not allowed", http.StatusBadGateway)
		return
	}

	upstreamURL := "wss://" + host + r.URL.RequestURI()
	proxyWebSocket(w, r, upstreamURL, "[ws]")
}

func filterWSHeaders(h http.Header) http.Header {
	out := http.Header{}
	for k, vals := range h {
		lk := strings.ToLower(k)
		switch lk {
		case "upgrade", "connection", "sec-websocket-key",
			"sec-websocket-version", "sec-websocket-extensions",
			"host", "content-length":
			continue
		}
		for _, v := range vals {
			out.Add(k, v)
		}
	}
	return out
}

func pumpWS(sess int64, dir string, src, dst *websocket.Conn, displayName string) {
	defer func() {
		_ = src.Close()
		_ = dst.Close()
	}()
	for {
		mt, payload, err := src.ReadMessage()
		if err != nil {
			if !websocket.IsCloseError(err, websocket.CloseNormalClosure, websocket.CloseGoingAway) &&
				!strings.Contains(err.Error(), "use of closed network connection") {
				log.Printf("[ws] read %s sess=%d: %v", dir, sess, err)
			}
			return
		}

		if _, seen := wsFirstFrame.LoadOrStore(fmt.Sprintf("%d-%s", sess, dir), true); !seen {
			preview := framePreview(payload, mt)
			if len(preview) > 180 {
				preview = preview[:180] + "..."
			}
			log.Printf("[ws] first %s sess=%d op=%s %d bytes preview=%q", dir, sess, wsOpcodeName(mt), len(payload), preview)
		}

		cfg := getCfg()
		patched := false
		out := payload
		svc := ""
		if mt == websocket.TextMessage {

			if dir == "c2s" {
				rememberWSRoute(sess, payload)
			}
			svc = wsFrameService(sess, payload)

			if cfg.nameSpoofReplaceActive() {
				if p, n := patchWSNameFields(out, cfg.nameSpoofDisplay(), dir); n > 0 {
					out = p
					patched = true
					log.Printf("[ws] %s sess=%d patched %d name field(s) (%d -> %d bytes)",
						dir, sess, n, len(payload), len(out))
					logWSPatch(sess, dir, "name_spoof", svc, len(payload), len(out))
				}
			}

			if dir == "s2c" && cfg.fakeRanksActive() {
				if p, ok := patchWSFakeRanks(out, cfg); ok {
					out = p
					patched = true
					logWSPatch(sess, dir, "fake_ranks", svc, len(payload), len(out))
				}
			}
		}

		_ = patched

		if err := dst.WriteMessage(mt, out); err != nil {
			log.Printf("[ws] write %s sess=%d: %v", dir, sess, err)
			return
		}
		if _, firstWrite := wsFirstFrame.LoadOrStore(fmt.Sprintf("%d-%s-written", sess, dir), true); !firstWrite {
			log.Printf("[ws] first %s sess=%d forwarded %d bytes ok", dir, sess, len(out))
		}
	}
}

var wsNameKeys = []string{
	"VerifiedPlayerName",
	"PlayerName",
	"DisplayName",
	"PlayerNickName",
	"NickName",
	"Name",
	"UserName",
}

func patchWSNameFields(body []byte, displayName, dir string) ([]byte, int) {
	cfg := getCfg()
	if displayName == "" {
		return body, 0
	}
	realName := cfg.nameSpoofRealName()
	replaceAll := cfg.nameSpoofReplaceAllPlayerNames()
	ownID := cfg.nameSpoofOwnPlayerID()
	if !replaceAll && ownID == "" && (realName == "" || realName == displayName) {
		return body, 0
	}
	if hdrEnd := bytes.Index(body, []byte("\r\n\r\n")); hdrEnd >= 0 {
		headers := body[:hdrEnd]
		jsonBody := body[hdrEnd+4:]
		if isLoadoutSensitiveFrame(headers, jsonBody) {
			return body, 0
		}
		if cfg.nameSpoofOwnPlayerID() == "" {
			if cid := wsHeaderValue(headers, "PsyConnectionID"); cid != "" {
				rememberOwnPlayerID(cid)
			}
		}
		ownID := cfg.nameSpoofOwnPlayerID()
		ctx := wsSwapContext(dir, headers, jsonBody)
		patched, n := patchJSONNameFields(jsonBody, displayName, realName, ownID, cfg.nameSpoofReplaceAllPlayerNames(), ctx)
		if n == 0 {
			return body, 0
		}
		if cfg.nameSpoofReplaceAllPlayerNames() {
			log.Printf("[name_spoof] WS replace_all_player_names %d field(s) -> %q (%s)", n, displayName, ctx)
		} else {
			log.Printf("[name_spoof] WS scrub %d× %q -> %q own=%q (%s)", n, realName, displayName, ownID, ctx)
		}
		newHeaders := resignWSHeaders(headers, patched)
		out := make([]byte, 0, len(newHeaders)+4+len(patched))
		out = append(out, newHeaders...)
		out = append(out, '\r', '\n', '\r', '\n')
		out = append(out, patched...)
		return out, n
	}

	if bodyLooksLoadoutSensitive(body) {
		return body, 0
	}
	ctx := wsSwapContext(dir, nil, body)
	return patchJSONNameFields(body, displayName, realName, cfg.nameSpoofOwnPlayerID(), cfg.nameSpoofReplaceAllPlayerNames(), ctx)
}

func patchJSONNameFields(body []byte, displayName, realName, ownID string, replaceAll bool, ctx string) ([]byte, int) {
	if displayName == "" {
		return body, 0
	}
	if replaceAll {
		return patchAllPlayerNamesOnWire(body, displayName, ctx)
	}
	n := 0
	out := body
	if ownID != "" {
		var c int
		out, c = patchNameFieldsForOwnID(out, ownID, displayName, "ws_own", ctx)
		n += c
	}
	if realName != "" && realName != displayName {
		var c int
		out, c = scrubRealNameAudited(out, realName, displayName, "ws_scrub", ctx)
		n += c
	}
	var c int
	out, c = patchBase64ContentNameFields(out, displayName, realName, ownID, ctx)
	n += c
	return out, n
}

func patchAllPlayerNamesOnWire(body []byte, displayName, ctx string) ([]byte, int) {
	n := 0
	out := body
	for _, key := range wsNameKeys {
		next, c := replaceJSONStringFieldAllAudited(out, key, displayName, "ws_lab", ctx)
		if c > 0 {
			out = next
			n += c
		}
	}
	return out, n
}

func resignWSHeaders(headers []byte, body []byte) []byte {
	psyTime := wsHeaderValue(headers, "PsyTime")
	got := wsHeaderValue(headers, "PsySig")
	if got == "" {
		got = wsHeaderValue(headers, "Psysignature")
	}
	if got == "" {
		return headers
	}
	var sig string
	if psyTime != "" {
		sig = resign(psyTime, body)
	} else {
		sig = resignWSRequest(body)
	}
	out, ok := replaceWSHeaderValue(headers, "PsySig", sig)
	if !ok {
		out, ok = replaceWSHeaderValue(headers, "Psysignature", sig)
	}
	if !ok {
		return headers
	}
	return out
}

func resignWSRequest(body []byte) string {
	h := hmac.New(sha256.New, []byte(psyReqKey))
	h.Write([]byte("-"))
	h.Write(body)
	return base64.StdEncoding.EncodeToString(h.Sum(nil))
}

func wsHeaderValue(headers []byte, key string) string {
	prefix := []byte(key + ":")
	lines := bytes.Split(headers, []byte("\r\n"))
	for _, line := range lines {
		if len(line) >= len(prefix) && bytes.EqualFold(line[:len(prefix)], prefix) {
			v := bytes.TrimSpace(line[len(prefix):])
			return string(v)
		}
	}
	return ""
}

func replaceWSHeaderValue(headers []byte, key, newValue string) ([]byte, bool) {
	prefix := []byte(key + ":")
	lines := bytes.Split(headers, []byte("\r\n"))
	changed := false
	for i, line := range lines {
		if len(line) >= len(prefix) && bytes.EqualFold(line[:len(prefix)], prefix) {
			lines[i] = append(append([]byte(nil), prefix...), ' ')
			lines[i] = append(lines[i], []byte(newValue)...)
			changed = true
			break
		}
	}
	if !changed {
		return headers, false
	}
	return bytes.Join(lines, []byte("\r\n")), true
}

func replaceJSONStringFieldAll(body []byte, key, newValue string) ([]byte, int) {
	encoded, ok := jsonStringContents(newValue)
	if !ok {
		return body, 0
	}
	prefix := []byte(`"` + key + `":"`)
	count := 0
	searchFrom := 0
	for {
		rel := bytes.Index(body[searchFrom:], prefix)
		if rel < 0 {
			break
		}
		i := searchFrom + rel
		valStart := i + len(prefix)
		j := jsonStringEnd(body, valStart)
		if j < 0 {
			break
		}
		if !bytes.Equal(body[valStart:j], encoded) {
			out := append([]byte(nil), body[:valStart]...)
			out = append(out, encoded...)
			out = append(out, body[j:]...)
			body = out
			count++
			searchFrom = valStart + len(encoded)
		} else {
			searchFrom = j
		}
	}
	return body, count
}

func wsOpcodeName(mt int) string {
	switch mt {
	case websocket.TextMessage:
		return "text"
	case websocket.BinaryMessage:
		return "binary"
	case websocket.CloseMessage:
		return "close"
	case websocket.PingMessage:
		return "ping"
	case websocket.PongMessage:
		return "pong"
	default:
		return fmt.Sprintf("op%d", mt)
	}
}

func framePreview(b []byte, mt int) string {
	if mt == websocket.TextMessage {
		s := string(b)
		if len(s) > 512 {
			return s[:512] + "..."
		}
		return s
	}
	n := len(b)
	if n > 64 {
		n = 64
	}
	return fmt.Sprintf("bin(%d)=%s", len(b), hex.EncodeToString(b[:n]))
}

func noteWSIncoming(remote, sni string) {
	if remote != "" {
		wsRecentHello.Store(remote, time.Now())
	}
	log.Printf("[ws] incoming ClientHello sni=%q from %s (%s)", sni, remote, peerProcess(remote))
}

func noteWSTLSReject(remote string, err error) {
	recentWS := false
	if remote != "" {
		if v, ok := wsRecentHello.Load(remote); ok {
			if t, ok := v.(time.Time); ok && time.Since(t) < 30*time.Second {
				recentWS = true
			}
		}
	}
	if !recentWS {
		return
	}

	n := wsTLSRejects.Add(1)
	log.Printf("[ws] REJECTED TLS handshake from %s (%s): %v", remote, peerProcess(remote), err)
	if n > 5 {
		return
	}
	log.Printf("[ws] Client rejected our leaf CA (websocket trust missing)")
	if getCfg().nameSpoofOpenSSLTrustActive() {
		log.Printf("[ws] openssl_trust is on but handshake still failed — re-check CA install")
	} else {
		log.Printf("[ws] enable openssl_trust only if websocket spoof requires it")
	}
}
