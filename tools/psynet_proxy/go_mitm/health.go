package main

import (
	"net/http"
	"os"
	"strings"
)

func hostsRedirectNote() string {
	if hostsHasConfigRedirect() {
		return "config.psynet.gg->127.0.0.1 OK"
	}
	return "config.psynet.gg NOT in hosts"
}

func hostsHasConfigRedirect() bool {
	root := os.Getenv("SystemRoot")
	if root == "" {
		root = `C:\Windows`
	}
	b, err := os.ReadFile(root + `\System32\drivers\etc\hosts`)
	if err != nil {
		return false
	}
	for _, line := range strings.Split(string(b), "\n") {
		t := strings.TrimSpace(line)
		if t == "" || strings.HasPrefix(t, "#") {
			continue
		}
		lower := strings.ToLower(t)
		if strings.Contains(lower, "config.psynet.gg") &&
			(strings.Contains(lower, "127.0.0.1") || strings.Contains(lower, "::1")) {
			return true
		}
	}
	return false
}

func publishRejectedExchange(r *http.Request, reason string) {
	if r == nil || r.URL == nil {
		return
	}
	ex, _ := r.Context().Value(capKey{}).(*Exchange)
	if ex != nil {
		ex.Status = "502 Bad Gateway"
		ex.StatusCode = 502
		ex.Error = reason
		captures.publish(ex)
		return
	}
	captures.publish(&Exchange{
		Method:     r.Method,
		Host:       r.Host,
		Path:       r.URL.Path,
		Query:      r.URL.RawQuery,
		ReqHeaders: headerMap(r.Header),
		Status:     "502 Bad Gateway",
		StatusCode: 502,
		Error:      reason,
	})
}
