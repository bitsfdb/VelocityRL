package main

import (
	"fmt"
	"os"
)

func writePIDFile() {
	pid := fmt.Sprintf("%d\n", os.Getpid())
	tmp := "proxy.pid.tmp"
	if err := os.WriteFile(tmp, []byte(pid), 0644); err != nil {
		return
	}
	_ = os.Remove("proxy.pid")
	_ = os.Rename(tmp, "proxy.pid")
}
