package main

import (
	"fmt"
	"net"
	"sync"
)

func listenLoopback443() (net.Listener, error) {
	addrs := []string{"127.0.0.1:443", "[::1]:443"}
	var opened []net.Listener
	for _, addr := range addrs {
		ln, err := net.Listen("tcp", addr)
		if err != nil {
			for _, o := range opened {
				_ = o.Close()
			}
			return nil, fmt.Errorf("listen %s: %w", addr, err)
		}
		opened = append(opened, ln)
	}
	if len(opened) == 1 {
		return opened[0], nil
	}
	return newMultiListener(opened...)
}

type multiListener struct {
	listeners []net.Listener
	connCh    chan net.Conn
	closeOnce sync.Once
	done      chan struct{}
}

func newMultiListener(listeners ...net.Listener) (*multiListener, error) {
	if len(listeners) == 0 {
		return nil, fmt.Errorf("multiListener: no listeners")
	}
	ml := &multiListener{
		listeners: listeners,
		connCh:    make(chan net.Conn),
		done:      make(chan struct{}),
	}
	for _, ln := range listeners {
		go ml.acceptLoop(ln)
	}
	return ml, nil
}

func (ml *multiListener) acceptLoop(ln net.Listener) {
	for {
		c, err := ln.Accept()
		if err != nil {
			select {
			case <-ml.done:
				return
			default:
				continue
			}
		}
		select {
		case ml.connCh <- c:
		case <-ml.done:
			_ = c.Close()
			return
		}
	}
}

func (ml *multiListener) Accept() (net.Conn, error) {
	select {
	case c, ok := <-ml.connCh:
		if !ok {
			return nil, net.ErrClosed
		}
		return c, nil
	case <-ml.done:
		return nil, net.ErrClosed
	}
}

func (ml *multiListener) Close() error {
	ml.closeOnce.Do(func() {
		close(ml.done)
		close(ml.connCh)
	})
	var first error
	for _, ln := range ml.listeners {
		if err := ln.Close(); err != nil && first == nil {
			first = err
		}
	}
	return first
}

func (ml *multiListener) Addr() net.Addr {
	if len(ml.listeners) > 0 {
		return ml.listeners[0].Addr()
	}
	return nil
}
