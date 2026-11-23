// This overlay keeps unreleased SDK declarations out of the module cache.
// Remove sdk-registry.go.txt and its replacement below when the sample SDK pin
// includes temporalio/sdk-go#2914. Keep the factory tests when removing the adapter.
package main

import (
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
)

func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

func run() error {
	if len(os.Args) < 2 {
		return fmt.Errorf("usage: go run ./testsupport/storagecontract test [go test arguments]")
	}
	_, source, _, _ := runtime.Caller(0)
	fixture := filepath.Dir(source)
	root := filepath.Clean(filepath.Join(fixture, "../.."))
	directory, err := os.MkdirTemp("", "nexgen-storage-contract-")
	if err != nil {
		return err
	}
	defer os.RemoveAll(directory)
	for _, name := range []string{"go.mod", "go.sum"} {
		contents, err := os.ReadFile(filepath.Join(root, name))
		if err != nil {
			return err
		}
		if err := os.WriteFile(filepath.Join(directory, name), contents, 0o600); err != nil {
			return err
		}
	}
	modfile := filepath.Join(directory, "go.mod")
	download := exec.Command("go", "mod", "download", "-modfile", modfile, "-json", "go.temporal.io/sdk")
	download.Dir = root
	download.Stderr = os.Stderr
	output, err := download.Output()
	if err != nil {
		return fmt.Errorf("download pinned SDK for storage contract overlay: %w\n%s", err, output)
	}
	var sdk struct{ Dir string }
	if err := json.Unmarshal(output, &sdk); err != nil {
		return fmt.Errorf("read pinned SDK download metadata: %w", err)
	}
	if sdk.Dir == "" {
		return fmt.Errorf("pinned SDK download did not report its source directory")
	}
	// Go forbids overlays beneath GOMODCACHE, so adapt a disposable copy.
	sdkCopy := filepath.Join(directory, "sdk")
	if err := os.CopyFS(sdkCopy, os.DirFS(sdk.Dir)); err != nil {
		return fmt.Errorf("copy pinned SDK for storage contract validation: %w", err)
	}
	edit := exec.Command("go", "mod", "edit", "-modfile", modfile, "-replace", "go.temporal.io/sdk="+sdkCopy)
	edit.Dir = root
	if output, err := edit.CombinedOutput(); err != nil {
		return fmt.Errorf("configure temporary SDK replacement: %w\n%s", err, output)
	}
	overlay := struct {
		Replace map[string]string
	}{
		Replace: map[string]string{
			filepath.Join(sdkCopy, "internal/nexus_operation_registry.go"):         filepath.Join(fixture, "sdk-registry.go.txt"),
			filepath.Join(root, "workflowservice/storage_target_contract_test.go"): filepath.Join(fixture, "registry_test.go.txt"),
		},
	}
	encoded, err := json.Marshal(overlay)
	if err != nil {
		return err
	}
	path := filepath.Join(directory, "overlay.json")
	if err := os.WriteFile(path, encoded, 0o600); err != nil {
		return err
	}
	arguments := append([]string{os.Args[1], "-modfile", modfile, "-overlay", path}, os.Args[2:]...)
	command := exec.Command("go", arguments...)
	command.Dir = root
	command.Stdin = os.Stdin
	command.Stdout = os.Stdout
	command.Stderr = os.Stderr
	if err := command.Run(); err != nil {
		return fmt.Errorf("Go storage contract validation: %w", err)
	}
	return nil
}
