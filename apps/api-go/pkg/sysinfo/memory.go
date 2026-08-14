// Copyright (c) 2025-2026 QuantumNous. All rights reserved.

// Package sysinfo reads process-scoped operating-system resource limits.
package sysinfo

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strconv"
	"strings"
)

const (
	procCgroupPath = "/proc/self/cgroup"
	cgroupRootPath = "/sys/fs/cgroup"
)

var ErrNoMemoryLimit = errors.New("process memory limit is unavailable")

type Memory struct {
	Current uint64
	High    uint64
	Max     uint64
	Limit   uint64
	Source  string
}

func (memory Memory) UsedPercent() float64 {
	if memory.Limit == 0 {
		return 0
	}
	return float64(memory.Current) * 100 / float64(memory.Limit)
}

// ReadProcessMemory returns cgroup-v2 usage and the tightest configured finite
// boundary. High and Max retain the individual controls; Limit is the lower
// finite value used for pressure decisions.
func ReadProcessMemory() (Memory, error) {
	cgroup, err := os.ReadFile(procCgroupPath)
	if err != nil {
		return Memory{}, fmt.Errorf("read process cgroup: %w", err)
	}
	return readMemory(cgroupRootPath, cgroup)
}

func readMemory(root string, procCgroup []byte) (Memory, error) {
	dir, err := cgroupDir(root, procCgroup)
	if err != nil {
		return Memory{}, err
	}
	current, finite, err := readLimit(filepath.Join(dir, "memory.current"))
	if err != nil || !finite {
		return Memory{}, fmt.Errorf("read cgroup memory.current: %w", err)
	}
	limits := []struct {
		name   string
		source string
	}{
		{name: "memory.high", source: "cgroup.memory.high"},
		{name: "memory.max", source: "cgroup.memory.max"},
	}
	memory := Memory{Current: current}
	for _, candidate := range limits {
		limit, isFinite, readErr := readLimit(filepath.Join(dir, candidate.name))
		if readErr != nil {
			return Memory{}, fmt.Errorf("read cgroup %s: %w", candidate.name, readErr)
		}
		if !isFinite || limit == 0 {
			continue
		}
		switch candidate.name {
		case "memory.high":
			memory.High = limit
		case "memory.max":
			memory.Max = limit
		}
		if memory.Limit == 0 || limit < memory.Limit {
			memory.Limit = limit
			memory.Source = candidate.source
		}
	}
	if memory.Limit == 0 {
		return Memory{}, ErrNoMemoryLimit
	}
	return memory, nil
}

func cgroupDir(root string, procCgroup []byte) (string, error) {
	var path string
	for _, line := range strings.Split(string(procCgroup), "\n") {
		fields := strings.SplitN(strings.TrimSpace(line), ":", 3)
		if len(fields) == 3 && fields[0] == "0" && fields[1] == "" {
			path = fields[2]
			break
		}
	}
	if path == "" || !strings.HasPrefix(path, "/") {
		return "", ErrNoMemoryLimit
	}
	cleanRoot := filepath.Clean(root)
	dir := filepath.Join(cleanRoot, strings.TrimPrefix(filepath.Clean(path), string(filepath.Separator)))
	if dir != cleanRoot && !strings.HasPrefix(dir, cleanRoot+string(filepath.Separator)) {
		return "", errors.New("process cgroup escapes cgroup root")
	}
	return dir, nil
}

func readLimit(path string) (value uint64, finite bool, err error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return 0, false, err
	}
	text := strings.TrimSpace(string(data))
	if text == "max" {
		return 0, false, nil
	}
	value, err = strconv.ParseUint(text, 10, 64)
	if err != nil {
		return 0, false, err
	}
	return value, true, nil
}
