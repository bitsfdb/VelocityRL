package main

import (
	"bytes"
	"context"
	"crypto/hmac"
	"crypto/sha256"
	"crypto/tls"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"io"
	"log"
	"net"
	"net/http"
	"net/http/httputil"
	"net/url"
	"os"
	"strings"
	"sync"
	"time"
)

const (
	targetURL  = "https://api.rlpp.psynet.gg"
	psyRespKey = "3b932153785842ac927744b292e40e52"
	psyReqKey  = "c338bd36fb8c42b1a431d30add939fc7"

	psyCDNKey = "cqhyz50f3c3j2pxhwo6b1kypxikah0wh"
)

var realIPs = map[string]string{
	"api.rlpp.psynet.gg": "34.54.194.77",
	"ws.rlpp.psynet.gg":  "34.149.116.40",
	"config.psynet.gg":   "34.160.180.65",
}

type TitleSwap struct {
	EquipTitleID     string     `json:"equip_title_id"`
	DisplayTitleID   string     `json:"display_title_id"`
	DisplayAsTitleID string     `json:"display_as_title_id"`
	SwappedTitleID   string     `json:"swapped_title_id"`
	CustomText       string     `json:"custom_text"`
	Category         string     `json:"category"`
	TitleColor       TitleColor `json:"title_color"`
}

type SpoofConfig struct {
	Enabled          bool                 `json:"enabled"`
	EquipTitleID     string               `json:"equip_title_id"`
	DisplayTitleID   string               `json:"display_title_id"`
	DisplayAsTitleID string               `json:"display_as_title_id"`
	SwappedTitleID   string               `json:"swapped_title_id"`
	CustomText       string               `json:"custom_text"`
	Category         string               `json:"category"`
	TitleColor       TitleColor           `json:"title_color"`
	CustomName       string               `json:"custom_name"`
	NameSpoof        NameSpoofConfig      `json:"name_spoof"`
	FakeRanks        FakeRanksConfig      `json:"fake_ranks"`
	PingSpoof        PingSpoofConfig      `json:"ping_spoof"`
	InventorySpoof   InventorySpoofConfig `json:"inventory_spoof"`
	LogoSpoof        LogoSpoofConfig      `json:"logo_spoof"`
	BlogSpoof        BlogSpoofConfig      `json:"blog_spoof"`
	CameraSpoof      CameraSpoofConfig    `json:"camera_spoof"`
	Swaps            []TitleSwap          `json:"swaps"`

	MainMenuBG string `json:"main_menu_bg"`

	Method string `json:"method"`
}

var (
	cfg     SpoofConfig
	cfgMu   sync.RWMutex
	cfgPath = "psynet_config.json"
	logFile *os.File
)

func loadCfg() {
	b, err := os.ReadFile(cfgPath)
	if err != nil {
		return
	}

	b = bytes.TrimPrefix(b, []byte{0xEF, 0xBB, 0xBF})
	var c SpoofConfig
	if err := json.Unmarshal(b, &c); err != nil {
		log.Printf("[cfg] FAILED to parse %s: %v (spoofs inactive until fixed)", cfgPath, err)
		return
	}

	if c.InventorySpoof.Enabled || len(c.InventorySpoof.Items) > 0 {
		log.Printf("[cfg] inventory_spoof forced OFF (feature removed)")
	}
	c.InventorySpoof = InventorySpoofConfig{}
	if c.PingSpoof.Enabled {
		log.Printf("[cfg] ping_spoof forced OFF (feature removed)")
	}
	c.PingSpoof = PingSpoofConfig{}
	cfgMu.Lock()
	cfg = c
	cfgMu.Unlock()
	syncLearnedPlayerIDFromCfg(c)
	logCfgLoaded(c)
}

func logCfgLoaded(c SpoofConfig) {
	log.Printf("[cfg] loaded: enabled=%v swaps=%d equip=%q text=%q cat=%q display=%q method=%q",
		c.Enabled, len(c.titleSwaps()), c.equipTitleID(), c.wantText(), c.wantCategory(), c.displayTitleID(), c.patchMethod())
	if c.logoSpoofActive() {
		log.Printf("[cfg] logo_spoof ON url=%q", strings.TrimSpace(c.LogoSpoof.LogoURL))
	} else if c.LogoSpoof.Enabled {
		log.Printf("[cfg] logo_spoof.enabled but logo_url empty - inactive")
	}
	if c.blogSpoofActive() {
		log.Printf("[cfg] blog_spoof ON MotD=%q", strings.TrimSpace(c.BlogSpoof.MotD))
	} else if c.BlogSpoof.Enabled {
		log.Printf("[cfg] blog_spoof.enabled but motd empty - inactive")
	}
	if c.cameraSpoofActive() {
		fov := c.CameraSpoof.FOV.resolved(defaultCameraFOV)
		h := c.CameraSpoof.Height.resolved(defaultCameraHeight)
		d := c.CameraSpoof.Distance.resolved(defaultCameraDistance)
		log.Printf("[cfg] camera_spoof ON FOV=[%.0f,%.0f] Height=[%.0f,%.0f] Distance=[%.0f,%.0f]",
			fov.Min, fov.Max, h.Min, h.Max, d.Min, d.Max)
	}
	if c.classPropNameActive() {
		log.Printf("[cfg] classprop_name ON display=%q", c.nameSpoofDisplay())
	} else if c.NameSpoof.ClassPropName {
		log.Printf("[cfg] classprop_name set but display_name empty - inactive")
	}
	if c.nameSpoofBrokerActive() {
		log.Printf("[cfg] name_spoof.broker ON display=%q (local RPC)", c.nameSpoofDisplay())
	} else if c.NameSpoof.Broker {
		log.Printf("[cfg] name_spoof.broker ON (local RPC + WS rewrite)")
	}
	if c.wsSpoofNeedsMITM() {
		log.Printf("[cfg] WS spoofs ON (via local broker)")
	}
	if c.nameSpoofReplaceAllPlayerNames() {
		log.Printf("[cfg] name_spoof.replace_all_player_names ON display=%q", c.nameSpoofDisplay())
	} else if c.nameSpoofReplaceActive() {
		log.Printf("[cfg] name_spoof WS scrub ON %q -> %q", c.nameSpoofRealName(), c.nameSpoofDisplay())
	}
	if c.fakeRanksActive() {
		log.Printf("[cfg] fake_ranks ON %s", fakeRanksSummary(c))
	} else if c.FakeRanks.Enabled {
		log.Printf("[cfg] fake_ranks.enabled but no default/playlists overrides - inactive")
	}

	if c.nameSpoofWSActive() {
		log.Printf("[cfg] name_spoof.websocket ON display=%q", c.nameSpoofDisplay())
	}
	if c.nameSpoofActive() {
		log.Printf("[cfg] name_spoof ENABLED display=%q (legacy path)", c.nameSpoofDisplay())
	} else if c.NameSpoof.Enabled {
		log.Printf("[cfg] name_spoof.enabled but display_name empty - inactive")
	} else if strings.TrimSpace(c.CustomName) != "" && !c.classPropNameActive() && !c.nameSpoofBrokerActive() {
		log.Printf("[cfg] custom_name=%q ignored (classprop_name/broker/enabled off)", c.CustomName)
	}
}

func watchCfg() {
	var lastMod int64 = -1
	for {
		if st, err := os.Stat(cfgPath); err == nil {
			m := st.ModTime().UnixNano()
			if m != lastMod {
				lastMod = m
				loadCfg()
			}
		}
		time.Sleep(500 * time.Millisecond)
	}
}

func getCfg() SpoofConfig {
	cfgMu.RLock()
	defer cfgMu.RUnlock()
	return cfg
}

func resign(psyTime string, body []byte) string {
	h := hmac.New(sha256.New, []byte(psyRespKey))
	h.Write([]byte(psyTime + "-"))
	h.Write(body)
	return base64.StdEncoding.EncodeToString(h.Sum(nil))
}

func hmacHex(key string, parts ...[]byte) string {
	h := hmac.New(sha256.New, []byte(key))
	for _, p := range parts {
		h.Write(p)
	}
	return base64.StdEncoding.EncodeToString(h.Sum(nil))
}

func resignConfigCDN(body []byte) string {
	return hmacHex(psyCDNKey, body)
}

func probeSignature(h http.Header, body []byte) {
	got := h.Get("Psysignature")
	if got == "" {
		got = h.Get("PsySig")
	}
	if got == "" {
		return
	}
	psyTime := h.Get("PsyTime")
	candidates := []struct{ name, sig string }{
		{"cdn-body", resignConfigCDN(body)},
		{"time-body", resign(psyTime, body)},
		{"body-only", hmacHex(psyRespKey, body)},
		{"dash-body", hmacHex(psyRespKey, []byte("-"), body)},
	}
	for _, c := range candidates {
		if c.sig == got {
			log.Printf("[sig] matched %s", c.name)
			return
		}
	}
	log.Printf("[sig] no match (PsyTime=%q got=%s)", psyTime, got)
}

func logMsg(dir string, msg []byte) {
	if logFile != nil {
		fmt.Fprintf(logFile, "%s %s\n", dir, msg)
	}
}

func dialReal(ctx context.Context, network, addr string) (net.Conn, error) {
	host, port, err := net.SplitHostPort(addr)
	if err != nil {
		return nil, err
	}
	ip, ok := realIPs[host]
	if !ok {
		d := net.Dialer{Timeout: 8 * time.Second}
		return d.DialContext(ctx, network, addr)
	}

	d := net.Dialer{Timeout: 5 * time.Second}
	var lastErr error
	for attempt := 1; attempt <= 3; attempt++ {
		conn, err := d.DialContext(ctx, network, net.JoinHostPort(ip, port))
		if err == nil {
			if attempt > 1 {
				log.Printf("[dial] %s (%s) recovered on attempt %d", host, ip, attempt)
			}
			return conn, nil
		}
		lastErr = err
		log.Printf("[dial] %s (%s) attempt %d failed: %v", host, ip, attempt, err)
		select {
		case <-time.After(800 * time.Millisecond):
		case <-ctx.Done():
			return nil, ctx.Err()
		}
	}
	return nil, lastErr
}

func main() {
	var err error

	if pl, e := os.OpenFile("psynet_proxy.log", os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0644); e == nil {
		log.SetOutput(io.MultiWriter(pl, bestEffortWriter{os.Stderr}))
	}
	if exe, e := os.Executable(); e == nil {
		if fi, e := os.Stat(exe); e == nil {
			log.Printf("[startup] psynet_proxy exe=%s modtime=%s size=%d pid=%d",
				exe, fi.ModTime().UTC().Format(time.RFC3339), fi.Size(), os.Getpid())
		}
	}
	writePIDFile()
	logFile, err = os.OpenFile("psynet_traffic.log", os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0644)
	if err != nil {
		log.Fatalf("log file: %v", err)
	}

	if p := strings.TrimSpace(os.Getenv("PSYNET_CONFIG")); p != "" {
		cfgPath = p
		log.Printf("[cfg] PSYNET_CONFIG=%s", cfgPath)
	}
	if d := strings.TrimSpace(os.Getenv("PSYNET_CAPTURE_DIR")); d != "" {
		captures.dir = d
		log.Printf("[cap] PSYNET_CAPTURE_DIR ignored in production (capture dumps removed)")
	}

	loadCfg()
	go watchCfg()
	captures.start()
	startRPCBroker()
	log.Printf("[cfg] AuthPlayer WS rewrite ALWAYS ON -> %s", defaultWSLocalURL)

	cert, err := tls.LoadX509KeyPair("server.crt", "server.key")
	if err != nil {
		log.Fatalf("TLS cert: %v", err)
	}
	var leafMu sync.Mutex
	leafCache := map[string]*tls.Certificate{}
	loadLeaf := func(host string) *tls.Certificate {
		leafMu.Lock()
		defer leafMu.Unlock()
		if c, ok := leafCache[host]; ok {
			return c
		}
		specific := fmt.Sprintf("leaf_%s.crt", host)
		keyPath := fmt.Sprintf("leaf_%s.key", host)
		if _, err := os.Stat(specific); err != nil {
			leafCache[host] = nil
			return nil
		}
		c, err := tls.LoadX509KeyPair(specific, keyPath)
		if err != nil {
			log.Printf("[tls] leaf load failed for %s: %v", specific, err)
			leafCache[host] = nil
			return nil
		}
		leafCache[host] = &c
		return &c
	}

	for _, h := range []string{"ws.rlpp.psynet.gg", "api.rlpp.psynet.gg", "config.psynet.gg"} {
		if c := loadLeaf(h); c != nil {
			log.Printf("[tls] preloaded leaf_%s.crt", h)
		}
	}

	tlsConfig := &tls.Config{

		MinVersion: tls.VersionTLS12,
		MaxVersion: tls.VersionTLS12,
		NextProtos: []string{"http/1.1"},
		CipherSuites: []uint16{
			tls.TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256,
			tls.TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384,
			tls.TLS_ECDHE_RSA_WITH_AES_128_CBC_SHA256,
			tls.TLS_ECDHE_RSA_WITH_AES_128_CBC_SHA,
			tls.TLS_ECDHE_RSA_WITH_AES_256_CBC_SHA,
			tls.TLS_RSA_WITH_AES_128_GCM_SHA256,
			tls.TLS_RSA_WITH_AES_256_GCM_SHA384,
			tls.TLS_RSA_WITH_AES_128_CBC_SHA256,
			tls.TLS_RSA_WITH_AES_128_CBC_SHA,
			tls.TLS_RSA_WITH_AES_256_CBC_SHA,
		},
		GetConfigForClient: func(hello *tls.ClientHelloInfo) (*tls.Config, error) {
			remote := ""
			if hello.Conn != nil {
				remote = hello.Conn.RemoteAddr().String()
			}
			log.Printf("[tls] clienthello sni=%q alpn=%v vers=%v from %s",
				hello.ServerName, hello.SupportedProtos, hello.SupportedVersions, remote)
			return nil, nil
		},
		GetCertificate: func(hello *tls.ClientHelloInfo) (*tls.Certificate, error) {
			name := strings.ToLower(hello.ServerName)
			remote := ""
			if hello.Conn != nil {
				remote = hello.Conn.RemoteAddr().String()
			}
			peer := peerProcess(remote)
			log.Printf("[tls] hello sni=%q alpn=%v vers=%v from %s",
				name, hello.SupportedProtos, hello.SupportedVersions, remote)
			log.Printf("[tls] peer %s", peer)

			emptySNI := name == "" || name == "127.0.0.1" || name == "::1" || name == "localhost"
			wsSNI := strings.Contains(name, "ws.rlpp.psynet.gg")
			c := getCfg()
			isRL := strings.Contains(strings.ToLower(peer), "rocketleague")

			if wsSNI {
				noteWSIncoming(remote, name)
				if !c.nameSpoofWSActive() {
					log.Printf("[ws] REJECTED sni=%q from %s - name_spoof.websocket is OFF (hosts should not redirect ws)", name, remote)
				}
			} else if emptySNI && c.nameSpoofWSActive() {

				noteWSIncoming(remote, name)
			}

			if emptySNI && (c.nameSpoofWSActive() || isRL) {
				if leaf := loadLeaf("ws.rlpp.psynet.gg"); leaf != nil {
					why := "empty SNI"
					if c.nameSpoofWSActive() && isRL {
						why = "empty SNI + RocketLeague + name_spoof.websocket"
					} else if c.nameSpoofWSActive() {
						why = "empty SNI + name_spoof.websocket (post-AuthPlayer WS dial)"
					} else if isRL {
						why = "empty SNI + RocketLeague peer"
					}
					log.Printf("[tls] serving leaf_ws.rlpp.psynet.gg.crt (%s)", why)
					return leaf, nil
				}
				log.Printf("[tls] leaf_ws.rlpp.psynet.gg.crt missing - falling back to server.crt (sni=%q)", name)
				log.Printf("[tls] serving server.crt (catch-all fallback, sni=%q)", name)
				return &cert, nil
			}
			if emptySNI {
				log.Printf("[tls] serving server.crt (catch-all, sni=%q peer=%s)", name, peer)
				return &cert, nil
			}
			if leaf := loadLeaf(name); leaf != nil {
				log.Printf("[tls] serving leaf_%s.crt", name)
				return leaf, nil
			}
			log.Printf("[tls] serving server.crt (no leaf for sni=%q)", name)
			return &cert, nil
		},
	}

	target, _ := url.Parse(targetURL)

	proxy := &httputil.ReverseProxy{
		Director: func(req *http.Request) {

			origHost := req.Host
			if _, ok := realIPs[origHost]; ok {
				req.URL.Scheme = "https"
				req.URL.Host = origHost
				req.Host = origHost
			} else {
				req.URL.Scheme = target.Scheme
				req.URL.Host = target.Host
				req.Host = target.Host
			}

			req.Header.Del("If-None-Match")
			req.Header.Del("If-Modified-Since")
			req.Header.Del("If-Match")

			var body []byte
			if req.Body != nil && req.Method != http.MethodGet && req.Method != http.MethodHead {
				body, _ = io.ReadAll(req.Body)
				req.Body = io.NopCloser(bytes.NewReader(body))
			}
			ex := &Exchange{
				Method:     req.Method,
				Host:       origHost,
				Path:       req.URL.Path,
				Query:      req.URL.RawQuery,
				ReqHeaders: headerMap(req.Header),
				ReqBody:    asRawJSON(body),
			}
			*req = *req.WithContext(context.WithValue(req.Context(), capKey{}, ex))
			log.Printf(">>> HTTP %s %s (Host: %s)", req.Method, req.URL.Path, origHost)
			logMsg(">>> "+origHost+" "+req.Method+" "+req.URL.Path+"\n", body)
		},
		Transport: &http.Transport{
			TLSClientConfig:       &tls.Config{InsecureSkipVerify: true, MinVersion: tls.VersionTLS12},
			DialContext:           dialReal,
			ResponseHeaderTimeout: 15 * time.Second,
			IdleConnTimeout:       30 * time.Second,
		},
		ErrorHandler: func(w http.ResponseWriter, r *http.Request, err error) {
			log.Printf("http: proxy error: %v", err)
			if ent, ok := staleLookup(r); ok {
				body := append([]byte(nil), ent.body...)
				hdr := ent.hdr.Clone()
				// Re-apply current logo/MotD/titles/camera against last-good body so a
				// stale cache from before those spoofs were enabled is not served forever.
				fake := &http.Response{Request: r, Header: hdr}
				if b, ok := maybePatchHTTP(fake, body); ok {
					body = b
					hdr = fake.Header.Clone()
				}
				for k, vs := range hdr {
					for _, v := range vs {
						w.Header().Add(k, v)
					}
				}
				w.Header().Set("X-VelocityRL-Cache", "last-good")
				w.Header().Set("Content-Length", fmt.Sprintf("%d", len(body)))
				w.WriteHeader(ent.status)
				_, _ = w.Write(body)
				key := staleKey(r)
				log.Printf("[cache] last-good %s (%d bytes, age %s, re-patched)", key, len(body), time.Since(ent.at).Round(time.Second))
				publishStaleExchange(r, staleEntry{status: ent.status, hdr: hdr, body: body, at: ent.at})
				return
			}
			if ex, ok := r.Context().Value(capKey{}).(*Exchange); ok && ex != nil {
				ex.Status = "502 Bad Gateway"
				ex.StatusCode = 502
				ex.Error = err.Error()
				captures.publish(ex)
			}
			http.Error(w, err.Error(), http.StatusBadGateway)
		},
		ModifyResponse: func(resp *http.Response) error {
			bodyBytes, _ := io.ReadAll(resp.Body)
			resp.Body.Close()

			probeSignature(resp.Header, bodyBytes)

			// Cache unpatched origin + origin headers so stale fallback can re-apply
			// the current logo/MotD/titles settings (not a previously patched body).
			originHdr := resp.Header.Clone()
			originBody := append([]byte(nil), bodyBytes...)
			patched := false
			if b, ok := maybePatchHTTP(resp, bodyBytes); ok {
				bodyBytes = b
				patched = true
			}
			if b, ok := maybePatchNameSpoofHTTP(resp, bodyBytes); ok {
				bodyBytes = b
				patched = true
			}

			resp.Header.Del("Etag")
			resp.Header.Del("ETag")
			resp.Header.Del("Age")
			resp.Header.Del("Alt-Svc")
			resp.Header.Del("Content-Encoding")
			resp.Header.Set("Cache-Control", "no-store, no-cache, must-revalidate")
			resp.Header.Set("Pragma", "no-cache")

			resp.Body = io.NopCloser(bytes.NewReader(bodyBytes))
			resp.ContentLength = int64(len(bodyBytes))
			resp.Header.Set("Content-Length", fmt.Sprintf("%d", len(bodyBytes)))

			respNote := ""
			if patched {
				respNote = " [patched]"
			}
			log.Printf("<<< HTTP %s (%d bytes)%s", resp.Status, len(bodyBytes), respNote)
			logMsg("<<< "+resp.Request.Host+" "+resp.Status+"\n", bodyBytes)

			if ex, _ := resp.Request.Context().Value(capKey{}).(*Exchange); ex != nil {
				ex.Status = resp.Status
				ex.StatusCode = resp.StatusCode
				ex.RespHeaders = headerMap(resp.Header)
				ex.RespBody = asRawJSON(bodyBytes)
				ex.RespBytes = len(bodyBytes)
				ex.Patched = patched
				if patched && !bytes.Equal(originBody, bodyBytes) {
					ex.RespBodyOrigin = asRawJSON(originBody)
				}
				captures.publish(ex)
			}
			staleSave(resp.Request, resp.StatusCode, originHdr, originBody)
			return nil
		},
	}

	mux := http.NewServeMux()
	mux.HandleFunc("/", func(w http.ResponseWriter, r *http.Request) {
		log.Printf("[http] %s %s host=%q from %s", r.Method, r.URL.Path, r.Host, r.RemoteAddr)
		c := getCfg()
		if isWebSocket(r) {
			if c.wsHostsMITMActive() {
				handleWebSocket(w, r)
				return
			}
			http.Error(w, "ws.rlpp not intercepted (enable WS spoofs or name_spoof.websocket)", http.StatusBadGateway)
			return
		}
		if !allowedMITMHost(r.Host, c) {
			log.Printf("[reject] host not intercepted: %q from %s", r.Host, r.RemoteAddr)
			publishRejectedExchange(r, "host not intercepted (config-only unless name_spoof.enabled)")
			http.Error(w, "host not intercepted (config-only unless name_spoof.enabled)", http.StatusBadGateway)
			return
		}
		proxy.ServeHTTP(w, r)
	})

	server := &http.Server{
		Addr:      ":443",
		Handler:   mux,
		TLSConfig: tlsConfig,
		ErrorLog:  log.New(&tlsErrorWriter{}, "", 0),
	}

	c0 := getCfg()
	hostsNote := hostsRedirectNote()
	switch {
	case c0.wsHostsMITMActive():
		log.Printf("Hosts expected: config.psynet.gg + ws (legacy websocket hosts)")
	case c0.psyNetURLBrokerRewriteActive():
		log.Printf("Hosts expected: config.psynet.gg only (local broker + WS)")
	case c0.nameSpoofActive():
		log.Printf("Hosts expected: config.psynet.gg + api (legacy name_spoof)")
	default:
		log.Printf("Hosts expected: config.psynet.gg only")
	}
	ln, err := listenLoopback443()
	if err != nil {
		if isAddrInUse(err) {
			log.Printf("listen 127.0.0.1:443: %v - another app owns loopback :443. Quit that app, run .\\stop_proxy.ps1, then retry.", err)
		} else {
			log.Printf("listen 127.0.0.1:443: %v", err)
		}
		log.Fatalf("[fatal] cannot bind loopback :443 (pid=%d hosts=%s) — proxy offline", os.Getpid(), hostsNote)
	}
	log.Printf("[ready] proxy pid=%d hosts=%s listening=127.0.0.1:443,::1:443 broker=http://%s ws=%s",
		os.Getpid(), hostsNote, brokerListenAddr(), defaultWSLocalURL)
	log.Fatal(server.Serve(tls.NewListener(acceptLogger{ln}, tlsConfig)))
}

type acceptLogger struct{ net.Listener }

func (l acceptLogger) Accept() (net.Conn, error) {
	c, err := l.Listener.Accept()
	if err != nil {
		return nil, err
	}
	remote := c.RemoteAddr().String()
	log.Printf("[tcp] accept %s (%s)", remote, peerProcess(remote))
	return c, nil
}

type bestEffortWriter struct{ w io.Writer }

func (b bestEffortWriter) Write(p []byte) (int, error) {
	_, _ = b.w.Write(p)
	return len(p), nil
}

type tlsErrorWriter struct{}

func (tlsErrorWriter) Write(p []byte) (int, error) {
	msg := string(p)
	log.Print(strings.TrimRight(msg, "\r\n"))
	if strings.Contains(msg, "TLS handshake error") {

		remote := ""
		if i := strings.Index(msg, "from "); i >= 0 {
			rest := msg[i+5:]
			if j := strings.IndexAny(rest, " :\r\n"); j > 0 {
				remote = rest[:j]
			}
		}

		noteWSTLSReject(remote, fmt.Errorf("%s", strings.TrimSpace(msg)))
	}
	return len(p), nil
}

func isAddrInUse(err error) bool {
	if err == nil {
		return false
	}
	msg := strings.ToLower(err.Error())
	return strings.Contains(msg, "only one usage") ||
		strings.Contains(msg, "address already in use") ||
		strings.Contains(msg, "wsaeaddrinuse")
}
