// Build inside the pinned BuildKit source tree with its committed vendor tree.
package main

import (
	"context"
	"encoding/binary"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"maps"
	"os"
	"os/signal"
	"sort"
	"strings"
	"sync"
	"syscall"
	"time"

	"github.com/distribution/reference"
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
	Requested string `json:"requested"`
	Effective string `json:"effective"`
	Digest    string `json:"digest"`
	Store     string `json:"store,omitempty"`
}

type recorder struct {
	gateway.Client
	mu      sync.Mutex
	queryMu sync.Mutex
	items   map[resolution]struct{}
	locals  map[localKey]localConfig
	bytes   int
}

type localKey struct {
	store, reference, session string
}

type localConfig struct {
	digest digest.Digest
	config []byte
}

var _ gateway.Client = (*recorder)(nil)

func identifier(value string) bool {
	if len(value) == 0 || len(value) > nameLimit {
		return false
	}
	for i := range value {
		if value[i] <= ' ' || value[i] >= 127 {
			return false
		}
	}
	return true
}

func (r *recorder) record(item resolution) error {
	dgst := digest.Digest(item.Digest)
	if dgst.Validate() != nil || dgst.Algorithm() != digest.SHA256 ||
		!identifier(item.Requested) || !identifier(item.Effective) ||
		(item.Store != "" && !identifier(item.Store)) {
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
	cost := len(encoded)
	if len(r.items) > 0 {
		cost++
	}
	if len(r.items) == itemLimit || r.bytes+cost > jsonLimit {
		return errors.New("discovery resolution limit")
	}
	r.items[item] = struct{}{}
	r.bytes += cost
	return nil
}

func (r *recorder) ResolveImageConfig(ctx context.Context, ref string, opt sourceresolver.Opt) (string, digest.Digest, []byte, error) {
	if local := opt.OCILayoutOpt; local != nil {
		return r.localConfig(ctx, ref, local.Store)
	}
	effective, dgst, config, err := r.Client.ResolveImageConfig(ctx, ref, opt)
	if err != nil {
		return effective, dgst, config, err
	}
	item := resolution{
		Requested: "docker-image://" + ref,
		Effective: "docker-image://" + effective,
		Digest:    dgst.String(),
	}
	if err := r.record(item); err != nil {
		return "", "", nil, err
	}
	return effective, dgst, config, nil
}

func (r *recorder) localConfig(ctx context.Context, ref string, store sourceresolver.ResolveImageConfigOptStore) (string, digest.Digest, []byte, error) {
	r.queryMu.Lock()
	defer r.queryMu.Unlock()
	if err := ctx.Err(); err != nil {
		return "", "", nil, err
	}
	key := localKey{store: store.StoreID, reference: ref, session: store.SessionID}
	r.mu.Lock()
	cached, found := r.locals[key]
	r.mu.Unlock()
	if found {
		return ref, cached.digest, cached.config, nil
	}
	named, err := reference.ParseNormalizedNamed(ref)
	if err != nil {
		return "", "", nil, err
	}
	digested, ok := named.(reference.Digested)
	if !ok || !identifier(store.StoreID) || !identifier(store.SessionID) {
		return "", "", nil, errors.New("invalid local metadata binding")
	}
	dgst := digested.Digest()
	if err := r.record(resolution{
		Requested: "oci-layout://" + ref,
		Effective: "oci-layout://" + ref,
		Digest:    dgst.String(),
		Store:     store.StoreID,
	}); err != nil {
		return "", "", nil, err
	}
	query, err := json.Marshal(struct {
		Store     string `json:"store"`
		Reference string `json:"reference"`
	}{store.StoreID, ref})
	if err != nil {
		return "", "", nil, err
	}
	if _, err := os.Stdout.Write(append(query, '\n')); err != nil {
		return "", "", nil, err
	}
	var length [4]byte
	if _, err := io.ReadFull(os.Stdin, length[:]); err != nil {
		return "", "", nil, err
	}
	size := binary.BigEndian.Uint32(length[:])
	if size == 0 || size > jsonLimit {
		return "", "", nil, errors.New("local metadata size limit")
	}
	config := make([]byte, size)
	if _, err := io.ReadFull(os.Stdin, config); err != nil {
		return "", "", nil, err
	}
	if !json.Valid(config) {
		return "", "", nil, errors.New("invalid local image config")
	}
	if err := ctx.Err(); err != nil {
		return "", "", nil, err
	}
	r.mu.Lock()
	r.locals[key] = localConfig{digest: dgst, config: config}
	r.mu.Unlock()
	return ref, dgst, config, nil
}

// SOURCE_DATE_EPOCH revisits the OCI source that NamedContext.Load resolved.
func (r *recorder) ResolveSourceMetadata(ctx context.Context, op *pb.SourceOp, opt sourceresolver.Opt) (*sourceresolver.MetaResponse, error) {
	ref, local := strings.CutPrefix(op.Identifier, "oci-layout://")
	if !local {
		return r.Client.ResolveSourceMetadata(ctx, op, opt)
	}
	key := localKey{
		store:     op.Attrs[pb.AttrOCILayoutStoreID],
		reference: ref,
		session:   op.Attrs[pb.AttrOCILayoutSessionID],
	}
	r.mu.Lock()
	cached, found := r.locals[key]
	r.mu.Unlock()
	if !found || len(op.Attrs) != 2 {
		return nil, errors.New("local source metadata was not previously resolved")
	}
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	return &sourceresolver.MetaResponse{
		Op: &pb.SourceOp{Identifier: op.Identifier, Attrs: maps.Clone(op.Attrs)},
		Image: &sourceresolver.ResolveImageResponse{
			Digest: cached.digest,
			Config: cached.config,
		},
	}, nil
}

func (r *recorder) snapshot() []resolution {
	r.mu.Lock()
	defer r.mu.Unlock()
	items := make([]resolution, 0, len(r.items))
	for item := range r.items {
		if item.Store == "" {
			items = append(items, item)
		}
	}
	sort.Slice(items, func(i, j int) bool {
		a, b := items[i], items[j]
		return a.Requested+"\x00"+a.Effective+"\x00"+a.Digest+"\x00"+a.Store <
			b.Requested+"\x00"+b.Effective+"\x00"+b.Digest+"\x00"+b.Store
	})
	return items
}

func discover() error {
	var locals, options repeated
	address := flag.String("addr", "unix:///run/buildkit/buildkitd.sock", "owned BuildKit daemon")
	policyPath := flag.String("source-policy-file", "", "host-written source policy (required)")
	outputPath := flag.String("output", "", "discovery receipt path (required)")
	flag.Var(&locals, "local", "BuildKit local name=directory; repeatable")
	flag.Var(&options, "opt", "native Dockerfile frontend key=value; repeatable")
	flag.Parse()
	if flag.NArg() != 0 || *policyPath == "" || *outputPath == "" {
		return errors.New("source-policy-file and output are required; positional arguments are unsupported")
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
	// The native conversion subrequest resolves config without solving RUN.
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
	// Keep anonymous token sessions without reading caller Docker credentials or
	// allowing the token-seed store to touch caller Docker state.
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
	_, err = client.Build(ctx, bkclient.SolveOpt{
		FrontendAttrs: attrs, LocalMounts: localMounts,
		SourcePolicy: &sourcePolicy, Session: []session.Attachable{anonymous},
	}, "pinfold-discover", func(ctx context.Context, client gateway.Client) (*gateway.Result, error) {
		// Interrupt metadata I/O when this solve, its deadline or a signal ends it.
		go func() {
			<-ctx.Done()
			os.Stdin.Close()
			os.Stdout.Close()
		}()
		observed = &recorder{
			Client: client, items: make(map[resolution]struct{}),
			locals: make(map[localKey]localConfig), bytes: len("{\"resolutions\":[]}\n"),
		}
		if _, err := builder.Build(ctx, observed); err != nil {
			return nil, err
		}
		return gateway.NewResult(), nil
	}, nil)
	if err != nil {
		return err
	}
	output, err := json.Marshal(struct {
		Resolutions []resolution `json:"resolutions"`
	}{observed.snapshot()})
	if err != nil {
		return err
	}
	if len(output)+1 > jsonLimit {
		return errors.New("discovery output size limit")
	}
	file, err := os.OpenFile(*outputPath, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o600)
	if err != nil {
		return err
	}
	if _, err := file.Write(append(output, '\n')); err != nil {
		file.Close()
		return err
	}
	return file.Close()
}

func main() {
	if err := discover(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
