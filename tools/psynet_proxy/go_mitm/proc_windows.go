//go:build windows

package main

import (
	"encoding/binary"
	"net"
	"path/filepath"
	"strconv"
	"syscall"
	"unsafe"
)

var (
	iphlpapi            = syscall.NewLazyDLL("iphlpapi.dll")
	getExtendedTcpTable = iphlpapi.NewProc("GetExtendedTcpTable")
	kernel32            = syscall.NewLazyDLL("kernel32.dll")
	openProcess         = kernel32.NewProc("OpenProcess")
	closeHandle         = kernel32.NewProc("CloseHandle")
	queryImageName      = kernel32.NewProc("QueryFullProcessImageNameW")
)

const (
	afInet                 = 2
	afInet6                = 23
	tcpTableOwnerPIDAll    = 5
	processQueryLimited    = 0x1000
)

func nboPort(v uint32) int {

	return int(binary.BigEndian.Uint16([]byte{byte(v), byte(v >> 8)}))
}

func peerProcess(remote string) string {
	_, portStr, err := net.SplitHostPort(remote)
	if err != nil {
		return "?"
	}
	port, err := strconv.Atoi(portStr)
	if err != nil {
		return "?"
	}

	if name := lookupTCP(port, afInet, 24); name != "" {
		return name
	}
	if name := lookupTCP(port, afInet6, 56); name != "" {
		return name
	}
	return "?"
}

func lookupTCP(port int, family uint32, rowSize int) string {
	var size uint32
	getExtendedTcpTable.Call(0, uintptr(unsafe.Pointer(&size)), 0, uintptr(family), tcpTableOwnerPIDAll, 0)
	if size == 0 {
		return ""
	}
	buf := make([]byte, size)
	r, _, _ := getExtendedTcpTable.Call(
		uintptr(unsafe.Pointer(&buf[0])),
		uintptr(unsafe.Pointer(&size)),
		0, uintptr(family), tcpTableOwnerPIDAll, 0,
	)
	if r != 0 {
		return ""
	}
	count := binary.LittleEndian.Uint32(buf[:4])
	off := 4
	for i := uint32(0); i < count && off+rowSize <= len(buf); i++ {
		row := buf[off : off+rowSize]
		off += rowSize
		var localPort, remotePort int
		var pid uint32
		if family == afInet {
			localPort = nboPort(binary.LittleEndian.Uint32(row[8:12]))
			remotePort = nboPort(binary.LittleEndian.Uint32(row[16:20]))
			pid = binary.LittleEndian.Uint32(row[20:24])
		} else {
			localPort = nboPort(binary.LittleEndian.Uint32(row[20:24]))
			remotePort = nboPort(binary.LittleEndian.Uint32(row[44:48]))
			pid = binary.LittleEndian.Uint32(row[52:56])
		}
		if localPort == port && (remotePort == 443 || remotePort == 0) {
			return processName(pid)
		}
	}
	return ""
}

func processName(pid uint32) string {
	if pid == 0 {
		return "pid0"
	}
	h, _, _ := openProcess.Call(processQueryLimited, 0, uintptr(pid))
	if h == 0 {
		return "pid" + strconv.FormatUint(uint64(pid), 10)
	}
	defer closeHandle.Call(h)
	var n uint32 = 260
	buf := make([]uint16, n)
	queryImageName.Call(h, 0, uintptr(unsafe.Pointer(&buf[0])), uintptr(unsafe.Pointer(&n)))
	path := syscall.UTF16ToString(buf)
	if path == "" {
		return "pid" + strconv.FormatUint(uint64(pid), 10)
	}
	return filepath.Base(path)
}
