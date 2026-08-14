// Copyright (c) 2025-2026 QuantumNous. All rights reserved.

package sysinfo

import (
	"errors"
	"os"
	"path/filepath"
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestReadMemoryUsesTighterCgroupHigh(t *testing.T) {
	root := t.TempDir()
	dir := filepath.Join(root, "system.slice", "lmm-api.service")
	require.NoError(t, os.MkdirAll(dir, 0o755))
	require.NoError(t, os.WriteFile(filepath.Join(dir, "memory.current"), []byte("1048576\n"), 0o644))
	require.NoError(t, os.WriteFile(filepath.Join(dir, "memory.max"), []byte("4194304\n"), 0o644))
	require.NoError(t, os.WriteFile(filepath.Join(dir, "memory.high"), []byte("2097152\n"), 0o644))

	memory, err := readMemory(root, []byte("0::/system.slice/lmm-api.service\n"))
	require.NoError(t, err)
	assert.Equal(t, uint64(1048576), memory.Current)
	assert.Equal(t, uint64(2097152), memory.High)
	assert.Equal(t, uint64(4194304), memory.Max)
	assert.Equal(t, uint64(2097152), memory.Limit)
	assert.Equal(t, "cgroup.memory.high", memory.Source)
	assert.Equal(t, 50.0, memory.UsedPercent())
}

func TestReadMemoryFallsBackToFiniteHigh(t *testing.T) {
	root := t.TempDir()
	require.NoError(t, os.WriteFile(filepath.Join(root, "memory.current"), []byte("1024"), 0o644))
	require.NoError(t, os.WriteFile(filepath.Join(root, "memory.max"), []byte("max"), 0o644))
	require.NoError(t, os.WriteFile(filepath.Join(root, "memory.high"), []byte("2048"), 0o644))

	memory, err := readMemory(root, []byte("0::/\n"))
	require.NoError(t, err)
	assert.Equal(t, uint64(2048), memory.High)
	assert.Zero(t, memory.Max)
	assert.Equal(t, "cgroup.memory.high", memory.Source)
	assert.Equal(t, 50.0, memory.UsedPercent())
}

func TestReadMemoryUsesMaxWhenItIsTighter(t *testing.T) {
	root := t.TempDir()
	require.NoError(t, os.WriteFile(filepath.Join(root, "memory.current"), []byte("1024"), 0o644))
	require.NoError(t, os.WriteFile(filepath.Join(root, "memory.high"), []byte("4096"), 0o644))
	require.NoError(t, os.WriteFile(filepath.Join(root, "memory.max"), []byte("2048"), 0o644))

	memory, err := readMemory(root, []byte("0::/\n"))
	require.NoError(t, err)
	assert.Equal(t, uint64(4096), memory.High)
	assert.Equal(t, uint64(2048), memory.Max)
	assert.Equal(t, uint64(2048), memory.Limit)
	assert.Equal(t, "cgroup.memory.max", memory.Source)
}

func TestReadMemoryRejectsMissingOrEscapingCgroup(t *testing.T) {
	_, err := readMemory(t.TempDir(), []byte("2:memory:/legacy\n"))
	assert.ErrorIs(t, err, ErrNoMemoryLimit)

	_, err = readMemory(t.TempDir(), []byte("0::/../../outside\n"))
	assert.True(t, errors.Is(err, ErrNoMemoryLimit) || err != nil)
}
