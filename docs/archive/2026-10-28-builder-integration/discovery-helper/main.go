// Source-only prototype. Build inside the pinned BuildKit v0.34.1 source tree.
package main

import (
	"context"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"os"
	"os/signal"
	"sort"
	"strings"
	"sync"
	"syscall"
	"time"

	dockerconfig "github.com/docker/cli/cli/config"
	"github.com/docker/cli/cli/config/types"
	bkclient "github.com/moby/buildkit/client"
	"github.com/moby/buildkit/client/llb/sourceresolver"
	buildopts "github.com/moby/buildkit/cmd/buildctl/build"
	"github.com/moby/buildkit/frontend/dockerfile/builder"
	gateway "github.com/moby/buildkit/frontend/gateway/client"
	"github.com/moby/buildkit/frontend/subrequests/convertllb"
	"github.com/moby/buildkit/session"
	"github.com/moby/buildkit/session/auth/authprovider"
	"github.com/moby/buildkit/solver/pb"
	policy "github.com/moby/buildkit/sourcepolicy/pb"
	digest "github.com/opencontainers/go-digest"
)

const (
	jsonLimit = 1 << 20
	itemLimit = 4096
	nameLimit = 8192
)

type repeated []string

func (v *repeated) String() string { return strings.Join(*v, ",") }
func (v *repeated) Set(value string) error {
	*v = append(*v, value)
	return nil
}

type resolution struct {
	Kind      string `json:"kind"`
	Requested string `json:"requested"`
	Effective string `json:"effective"`
	Digest    string `json:"digest"`
	Store     string `json:"store,omitempty"`
}

type recorder struct {
	gateway.Client
	mu    sync.Mutex
	items map[resolution]struct{}
	bytes int
}

var _ gateway.Client = (*recorder)(nil)

func (r *recorder) record(item resolution) error {
	dgst := digest.Digest(item.Digest)
	if dgst.Validate() != nil || dgst.Algorithm() != digest.SHA256 ||
		len(item.Requested) > nameLimit || len(item.Effective) > nameLimit || len(item.Store) > nameLimit {
		return errors.New("discovery resolution exceeds supported shape")
	}
	r.mu.Lock()
	defer r.mu.Unlock()
	if _, exists := r.items[item]; exists {
		return nil
	}
	encoded, err := json.Marshal(item)
	if err != nil {
		return err
	}
	if len(r.items) == itemLimit || r.bytes+len(encoded)+1 > jsonLimit {
		return errors.New("discovery resolution limit")
	}
	r.items[item] = struct{}{}
	r.bytes += len(encoded) + 1
	return nil
}

func (r *recorder) ResolveImageConfig(ctx context.Context, ref string, opt sourceresolver.Opt) (string, digest.Digest, []byte, error) {
	effective, dgst, config, err := r.Client.ResolveImageConfig(ctx, ref, opt)
	if err != nil {
		return effective, dgst, config, err
	}
	item := resolution{Kind: "docker-image", Digest: dgst.String()}
	if opt.OCILayoutOpt != nil {
		item.Kind = "oci-layout"
		item.Store = opt.OCILayoutOpt.Store.StoreID
	}
	item.Requested = item.Kind + "://" + ref
	item.Effective = item.Kind + "://" + effective
	if err := r.record(item); err != nil {
		return "", "", nil, err
	}
	return effective, dgst, config, nil
}

func (r *recorder) ResolveSourceMetadata(ctx context.Context, op *pb.SourceOp, opt sourceresolver.Opt) (*sourceresolver.MetaResponse, error) {
	requested := op.Identifier
	result, err := r.Client.ResolveSourceMetadata(ctx, op, opt)
	if err != nil || result == nil || result.Image == nil {
		return result, err
	}
	if result.Op == nil {
		return nil, errors.New("image resolution lacks effective source")
	}
	kind := ""
	for _, prefix := range []string{"docker-image", "oci-layout"} {
		if strings.HasPrefix(result.Op.Identifier, prefix+"://") {
			kind = prefix
		}
	}
	if kind == "" {
		return nil, errors.New("image resolution has unsupported source kind")
	}
	item := resolution{
		Kind: kind, Requested: requested, Effective: result.Op.Identifier,
		Digest: result.Image.Digest.String(),
	}
	if opt.OCILayoutOpt != nil {
		item.Store = opt.OCILayoutOpt.Store.StoreID
	}
	if err := r.record(item); err != nil {
		return nil, err
	}
	return result, nil
}

func (r *recorder) snapshot() []resolution {
	r.mu.Lock()
	defer r.mu.Unlock()
	items := make([]resolution, 0, len(r.items))
	for item := range r.items {
		items = append(items, item)
	}
	sort.Slice(items, func(i, j int) bool {
		a, b := items[i], items[j]
		return a.Kind+"\x00"+a.Requested+"\x00"+a.Effective+"\x00"+a.Digest+"\x00"+a.Store <
			b.Kind+"\x00"+b.Requested+"\x00"+b.Effective+"\x00"+b.Digest+"\x00"+b.Store
	})
	return items
}

func discover() error {
	var locals, layouts, options repeated
	address := flag.String("addr", "unix:///run/buildkit/buildkitd.sock", "owned BuildKit daemon")
	policyPath := flag.String("source-policy-file", "", "host-written source policy (required)")
	flag.Var(&locals, "local", "BuildKit local name=directory; repeatable")
	flag.Var(&layouts, "oci-layout", "BuildKit OCI store name=directory; repeatable")
	flag.Var(&options, "opt", "native Dockerfile frontend key=value; repeatable")
	flag.Parse()
	if flag.NArg() != 0 || *policyPath == "" {
		return errors.New("source-policy-file is required and positional arguments are unsupported")
	}
	policyFile, err := os.Open(*policyPath)
	if err != nil {
		return err
	}
	defer policyFile.Close()
	policyBytes, err := io.ReadAll(io.LimitReader(policyFile, jsonLimit+1))
	if err != nil {
		return err
	}
	if len(policyBytes) > jsonLimit {
		return errors.New("source policy size limit")
	}
	var sourcePolicy policy.Policy
	if err := json.Unmarshal(policyBytes, &sourcePolicy); err != nil {
		return err
	}
	attrs, err := buildopts.ParseOpt(options)
	if err != nil {
		return err
	}
	// These are fixed capabilities, regardless of supplied frontend options.
	attrs["requestid"] = convertllb.RequestConvertLLB
	attrs["frontend.caps"] = "moby.buildkit.frontend.subrequests"
	attrs["cmdline"] = ""
	attrs["platform"] = "linux/amd64"
	attrs["build-arg:TARGETARCH"] = "amd64"
	localMounts, err := buildopts.ParseLocal(locals)
	if err != nil {
		return err
	}
	if localMounts["context"] == nil || localMounts["dockerfile"] == nil {
		return errors.New("context and dockerfile local mounts are required")
	}
	stores, err := buildopts.ParseOCILayout(layouts)
	if err != nil {
		return err
	}
	// Keep buildctl's anonymous token session capability without reading caller
	// credentials or allowing its token-seed store to touch caller Docker state.
	authDir, err := os.MkdirTemp("", "pinfold-discover-auth-")
	if err != nil {
		return err
	}
	defer os.RemoveAll(authDir)
	dockerconfig.SetDir(authDir)
	anonymous := authprovider.NewDockerAuthProvider(authprovider.DockerAuthProviderConfig{
		AuthConfigProvider: func(context.Context, string, []string, authprovider.ExpireCachedAuthCheck) (types.AuthConfig, error) {
			return types.AuthConfig{}, nil
		},
	})
	ctx, stop := signal.NotifyContext(context.Background(), syscall.SIGINT, syscall.SIGTERM)
	defer stop()
	ctx, cancel := context.WithTimeout(ctx, 300*time.Second)
	defer cancel()
	client, err := bkclient.New(ctx, *address)
	if err != nil {
		return err
	}
	defer client.Close()
	var observed *recorder
	var converted json.RawMessage
	_, err = client.Build(ctx, bkclient.SolveOpt{
		FrontendAttrs: attrs, LocalMounts: localMounts, OCIStores: stores,
		SourcePolicy: &sourcePolicy, Session: []session.Attachable{anonymous},
	}, "pinfold-discover-prototype", func(ctx context.Context, client gateway.Client) (*gateway.Result, error) {
		observed = &recorder{Client: client, items: make(map[resolution]struct{})}
		result, err := builder.Build(ctx, observed)
		if err != nil {
			return nil, err
		}
		converted = result.Metadata["result.json"]
		if len(converted) > jsonLimit || !json.Valid(converted) {
			return nil, errors.New("native conversion result size or JSON shape")
		}
		var definition struct {
			Def map[string]json.RawMessage `json:"def"`
		}
		if err := json.Unmarshal(converted, &definition); err != nil {
			return nil, err
		}
		if definition.Def == nil || len(definition.Def) > itemLimit {
			return nil, errors.New("native conversion operation limit")
		}
		return result, nil
	}, nil)
	if err != nil {
		return err
	}
	output, err := json.Marshal(struct {
		ConvertLLB  json.RawMessage `json:"convert_llb"`
		Resolutions []resolution    `json:"resolutions"`
	}{converted, observed.snapshot()})
	if err != nil {
		return err
	}
	if len(output)+1 > jsonLimit {
		return errors.New("combined discovery output size limit")
	}
	_, err = os.Stdout.Write(append(output, '\n'))
	return err
}

func main() {
	if err := discover(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
