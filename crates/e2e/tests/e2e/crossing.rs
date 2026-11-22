//! Guarantee 10: disk-project branches and tags cross only through the Git
//! remote.

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use e2e::{Exec, Project, TestEnv, default_image, tree};

/// Git's client-side hook names (githooks(5)).
const HOOKS: [&str; 18] = [
    "applypatch-msg",
    "pre-applypatch",
    "post-applypatch",
    "pre-commit",
    "pre-merge-commit",
    "prepare-commit-msg",
    "commit-msg",
    "post-commit",
    "pre-rebase",
    "post-checkout",
    "post-merge",
    "pre-push",
    "pre-auto-gc",
    "post-rewrite",
    "sendemail-validate",
    "fsmonitor-watchman",
    "post-index-change",
    "reference-transaction",
];

/// The box's PATH: a home directory the box writes first, then the image's.
const BOX_PATH: &str =
    "/home/pinfold/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

/// The box's `git` on that PATH. The fetch runs whatever `git` the box
/// runs; while `/home/pinfold/.hostile` holds an object id, this one adds a
/// branch `stolen` at that id to the branch listing. While `.prereq` holds
/// one, the bundle names it as a prerequisite. While `.broken` holds one,
/// the bundle's pack also carries a commit no head reaches whose tree is
/// that id. Otherwise it is the image's git.
const HOSTILE_GIT: &str = r#"#!/bin/sh
h=/home/pinfold
if [ -s $h/.hostile ] && [ "$3" = for-each-ref ] && [ "$5" = refs/heads/ ]; then
  case $4 in
  *objectname*)
    /usr/bin/git "$@" || exit
    printf '%s refs/heads/stolen\n' "$(cat $h/.hostile)"
    exit ;;
  esac
fi
if [ "$5 $6" = "bundle create" ] && { [ -s $h/.prereq ] || [ -s $h/.broken ]; }; then
  set -e
  revs=$(cat)
  b=$(mktemp)
  printf '%s\n' "$revs" | /usr/bin/git "$@" >"$b"
  if [ -s $h/.prereq ]; then
    head -n1 "$b"
    printf -- '-%s\n' "$(cat $h/.prereq)"
    tail -n +2 "$b"
  else
    commit=$(printf 'tree %s\nauthor b <b@b> 0 +0000\ncommitter b <b@b> 0 +0000\n\nbroken\n' "$(cat $h/.broken)" |
      /usr/bin/git -C "$2" hash-object -t commit -w --stdin)
    sed '/^$/q' "$b"
    { printf '%s\n' "$revs" | /usr/bin/git -C "$2" rev-list --objects --stdin | cut -d' ' -f1; echo "$commit"; } |
      /usr/bin/git -C "$2" pack-objects -q --stdout
  fi
  rm "$b"
  exit
fi
exec /usr/bin/git "$@"
"#;

#[test]
fn disk_project_branches_and_tags_cross_only_through_the_git_remote() {
    // Sabotage (run): in core/crossing.rs's `fetch_cancellable`, skip the
    // branch check `!carried && !reached(oid)?`. The hostile fetch then
    // exits 0 and writes `refs/remotes/box/stolen` at the host-only commit.
    // Sabotage (run): in `fetch_cancellable`'s `kept_out`, drop the
    // `host_names` case-folded match. The forced tag fetch then writes the
    // box's `Light`.
    // Sabotage (run): drop `fetch_cancellable`'s `rev-list` connectivity
    // check of the sent objects. The fetch with the broken commit exits 0.
    // Sabotage (run): run `bundle verify` without `quarantine.env()`. The
    // promisor's commit lands in `.git/objects`.
    // The legitimate fetch, from the host as a partial clone, and the
    // host's own hooks are the positive controls.
    default_image();
    let env = TestEnv::new("crossing");
    let project = Project::new(&env, "crossing");
    fs::write(
        project.config_dir().join("pinfold.toml"),
        format!(
            "mounts = [{{ host = \".\", kind = \"disk\" }}]\nenv = {{ PATH = \"{BOX_PATH}\" }}\n"
        ),
    )
    .unwrap();
    let git = |args: &[&str]| ok(project.git(args), args);
    let in_box = |args: &[&str]| {
        let argv: Vec<&str> = ["--", "sh", "-c"].iter().chain(args).copied().collect();
        ok(project.pinfold(&argv), args)
    };

    // The host: main at c1 with an annotated tag v1 and a lightweight tag
    // light; a branch the box never gets.
    fs::write(project.root().join("readme"), "one\n").unwrap();
    git(&["add", "readme"]);
    git(&["commit", "-q", "-m", "c1"]);
    git(&["tag", "-a", "v1", "-m", "v1"]);
    git(&["tag", "light"]);
    git(&["switch", "-q", "-c", "secret"]);
    git(&["commit", "-q", "--allow-empty", "-m", "host only"]);
    let secret = git(&["rev-parse", "HEAD"]);
    git(&["switch", "-q", "main"]);
    let c1 = git(&["rev-parse", "main"]);

    // A fresh start writes no host Git configuration and leaves the
    // project disk empty.
    let config = project.root().join(".git/config");
    let host_config = fs::read(&config).unwrap();
    ok(project.pinfold(&["start"]), &["start"]);
    assert_eq!(fs::read(&config).unwrap(), host_config);
    assert_eq!(in_box(&["ls -A"]), "");

    // The first push creates the box repository, holding only host/main.
    git(&[
        "remote",
        "add",
        "box",
        &format!("pinfold::{}", project.root().display()),
    ]);
    git(&["push", "-q", "box", "main"]);
    assert_eq!(
        box_refs(&in_box),
        BTreeMap::from([("refs/remotes/host/main".to_string(), c1.clone())])
    );

    // The host's own hooks write to `host-ran`, installed before any fetch
    // so nothing from the box can be overwritten unseen.
    let planted = env.root().join("planted");
    let host_ran = env.root().join("host-ran");
    fs::create_dir(&planted).unwrap();
    fs::create_dir(&host_ran).unwrap();
    let hooks = project.root().join(".git/hooks");
    assert!(!hooks.exists(), "git init --template= made no hooks");
    fs::create_dir(&hooks).unwrap();
    for hook in ["post-merge", "post-checkout"] {
        let path = hooks.join(hook);
        let marker = shell_quote(&host_ran.join(hook).display().to_string());
        fs::write(&path, format!("#!/bin/sh\ntouch {marker}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let host_hooks = files(&hooks);

    // The box checks main out and commits hooks under every client-side
    // name in .githooks/ and .husky/, which its own core.hooksPath names.
    // Each would leave a file named for it in `planted` on the host. In the
    // box, where that path does not exist, each succeeds and leaves nothing:
    // a failing reference-transaction hook would refuse the box's own ref
    // updates, the push's included. Its tags: `box-new`, new; `light` and
    // `Light`, named like the host's `light` exactly and but for case, at
    // other commits.
    let planted_quoted = shell_quote(&planted.display().to_string());
    let mut setup = vec![
        "set -e
         mkdir -p /home/pinfold/bin; printf %s \"$1\" > /home/pinfold/bin/git; chmod 755 /home/pinfold/bin/git
         marker=$2; shift 2
         git switch -q -c main host/main
         mkdir .githooks .husky
         for name do
           printf '#!/bin/sh\\ntouch %s/%s 2>/dev/null\\nexit 0\\n' \"$marker\" \"$name\" > .githooks/$name
           cp .githooks/$name .husky/$name
         done
         chmod 755 .githooks/* .husky/*
         git add .githooks .husky
         git -c core.hooksPath=/dev/null commit -q -m hooks
         git tag -a box-new -m box-new
         git tag light
         git switch -q -c feature
         echo feature > feature
         git add feature
         git -c core.hooksPath=/dev/null commit -q -m feature
         git tag Light
         git switch -q main
         git config core.hooksPath .githooks",
        "sh",
        HOSTILE_GIT,
        &planted_quoted,
    ];
    setup.extend(HOOKS);
    in_box(&setup);
    let box_ids = in_box(&["git rev-parse main feature box-new light Light"]);
    let [box_main, box_feature, box_tag, box_light, box_upper] =
        <[&str; 5]>::try_from(box_ids.lines().collect::<Vec<_>>()).unwrap();

    // (a) A hostile listing names `stolen` at the host-only commit, beside
    // a real bundle. The fetch fails naming it and changes nothing.
    let refs = || git(&["for-each-ref", "--format=%(objectname) %(refname)"]);
    let (refs_before, objects_before) = (refs(), tree(&project.root().join(".git/objects")));
    in_box(&[
        "printf %s \"$1\" > /home/pinfold/.hostile",
        "sh",
        secret.as_str(),
    ]);
    let hostile = project.git(&["fetch", "box"]);
    assert_ne!(hostile.code, 0, "{hostile:?}");
    assert!(
        hostile
            .stderr
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
            .any(|word| word == "stolen"),
        "{}",
        hostile.stderr
    );
    assert_eq!(refs(), refs_before);
    assert_eq!(tree(&project.root().join(".git/objects")), objects_before);
    in_box(&["rm /home/pinfold/.hostile"]);

    // The box's pack also carries a commit no head reaches, whose tree
    // exists nowhere. The fetch fails naming that tree and changes nothing.
    let nowhere = env.root().join("nowhere");
    fs::write(&nowhere, "no such tree\n").unwrap();
    let missing = git(&["hash-object", nowhere.to_str().unwrap()]);
    in_box(&[
        "printf %s \"$1\" > /home/pinfold/.broken",
        "sh",
        missing.as_str(),
    ]);
    let broken = project.git(&["fetch", "box"]);
    assert_ne!(broken.code, 0, "{broken:?}");
    assert!(broken.stderr.contains(&missing), "{}", broken.stderr);
    assert_eq!(refs(), refs_before);
    assert_eq!(tree(&project.root().join(".git/objects")), objects_before);
    in_box(&["rm /home/pinfold/.broken"]);

    // The host becomes a partial clone whose promisor, a local repository,
    // holds a commit the host lacks. The listing names `stolen` there and
    // the bundle names it as a prerequisite, which `git bundle verify`
    // fetches from the promisor. The fetch fails naming `stolen`, and
    // `.git/objects` is unchanged. The host stays a partial clone for (b).
    let promisor = env.root().join("promisor");
    let promisor = promisor.to_str().unwrap();
    git(&["clone", "-q", "--bare", ".", promisor]);
    let promised = git(&[
        "-C",
        promisor,
        "commit-tree",
        "-m",
        "promised",
        "main^{tree}",
    ]);
    git(&[
        "-C",
        promisor,
        "update-ref",
        "refs/heads/promised",
        &promised,
    ]);
    git(&["config", "remote.promisor.url", promisor]);
    git(&["config", "remote.promisor.promisor", "true"]);
    git(&["config", "extensions.partialClone", "promisor"]);
    let objects_before = tree(&project.root().join(".git/objects"));
    in_box(&[
        "printf %s \"$1\" > /home/pinfold/.hostile; printf %s \"$1\" > /home/pinfold/.prereq",
        "sh",
        promised.as_str(),
    ]);
    let prerequisite = project.git(&["fetch", "box"]);
    assert_ne!(prerequisite.code, 0, "{prerequisite:?}");
    assert!(
        prerequisite
            .stderr
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
            .any(|word| word == "stolen"),
        "{}",
        prerequisite.stderr
    );
    assert_eq!(refs(), refs_before);
    assert_eq!(tree(&project.root().join(".git/objects")), objects_before);
    in_box(&["rm /home/pinfold/.hostile /home/pinfold/.prereq"]);
    // The promisor is live: the host's own lookup fetches the commit.
    git(&["cat-file", "-e", &promised]);

    // (b) A valid listing carries `light` and `Light` at other objects. A
    // forced, pruning tag fetch moves and deletes no host branch or tag and
    // writes no `Light`; the box's branches and its new tag land.
    let named = |prefix: &str| {
        git(&["for-each-ref", "--format=%(refname) %(objectname)", prefix])
            .lines()
            .map(|line| {
                let (name, id) = line.split_once(' ').unwrap();
                (name.to_string(), id.to_string())
            })
            .collect::<BTreeMap<_, _>>()
    };
    let (host_tags, host_heads) = (named("refs/tags/"), named("refs/heads/"));
    assert_eq!(host_tags.len(), 2, "{host_tags:?}");
    let host_light = host_tags["refs/tags/light"].clone();
    assert!(
        host_light != box_light && host_light != box_upper,
        "{host_light}"
    );
    git(&[
        "fetch",
        "-q",
        "--force",
        "--tags",
        "--prune",
        "--prune-tags",
        "box",
    ]);
    let mut expected = host_tags.clone();
    expected.insert("refs/tags/box-new".into(), box_tag.into());
    assert_eq!(named("refs/tags/"), expected);
    assert_eq!(named("refs/heads/"), host_heads);
    assert_eq!(git(&["rev-parse", "refs/tags/light"]), host_light);
    assert_eq!(git(&["rev-parse", "refs/remotes/box/main"]), box_main);
    assert_eq!(git(&["rev-parse", "refs/remotes/box/feature"]), box_feature);
    assert_eq!(names(&host_ran), Vec::<String>::new());

    // Twice: the host fetches the box's newest main, merges and checks
    // out. Its own hooks run; nothing the box planted does.
    let mut fetched = box_main.to_string();
    for round in ["one", "two"] {
        if round == "two" {
            in_box(&[
                "echo two > two && git add two && git -c core.hooksPath=/dev/null commit -q -m two",
            ]);
            fetched = in_box(&["git rev-parse main"]);
            assert_ne!(fetched, box_main);
        }
        git(&["fetch", "-q", "box"]);
        assert_eq!(git(&["rev-parse", "box/main"]), fetched, "round {round}");
        git(&["merge", "-q", "box/main"]);
        git(&["checkout", "-q", "-b", round]);
        git(&["checkout", "-q", "main"]);
    }
    assert_eq!(git(&["rev-parse", "main"]), fetched);
    assert_eq!(names(&host_ran), ["post-checkout", "post-merge"]);
    assert_eq!(names(&planted), Vec::<String>::new());
    assert_eq!(project.git(&["config", "--get", "core.hooksPath"]).code, 1);

    // With push.followTags, `git push box main` lands main in the box's
    // host/main and its annotated tags as box tags; the box's branches,
    // own tags and working tree stay.
    fs::write(project.root().join("host"), "host\n").unwrap();
    git(&["add", "host"]);
    git(&["commit", "-q", "-m", "c2"]);
    git(&["tag", "-a", "rel", "-m", "rel"]);
    let worktree = "find . -path ./.git -prune -o -type f -exec sha256sum {} + | sort";
    let (box_before, tree_before) = (box_refs(&in_box), in_box(&[worktree]));
    git(&["-c", "push.followTags=true", "push", "-q", "box", "main"]);
    let mut expected = box_before.clone();
    expected.insert("refs/remotes/host/main".into(), git(&["rev-parse", "main"]));
    expected.insert("refs/tags/rel".into(), git(&["rev-parse", "refs/tags/rel"]));
    expected.insert("refs/tags/v1".into(), git(&["rev-parse", "refs/tags/v1"]));
    assert_eq!(box_refs(&in_box), expected);
    assert_eq!(in_box(&[worktree]), tree_before);
    assert_eq!(names(&planted), Vec::<String>::new());
    assert_eq!(files(&hooks), host_hooks);
}

/// `text` as one shell word.
fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// Every file in `dir`, by name, with its bytes.
fn files(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fs::read_dir(dir)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            let name = entry.file_name().into_string().unwrap();
            (name, fs::read(entry.path()).unwrap())
        })
        .collect()
}

/// Exit 0, its stdout trimmed.
fn ok(out: Exec, what: &[&str]) -> String {
    assert_eq!(out.code, 0, "{what:?}: {out:?}");
    out.stdout.trim().to_string()
}

/// Every ref in the box's repository, by name.
fn box_refs(in_box: &dyn Fn(&[&str]) -> String) -> BTreeMap<String, String> {
    in_box(&["git for-each-ref --format='%(refname) %(objectname)'"])
        .lines()
        .map(|line| {
            let (name, id) = line.split_once(' ').unwrap();
            (name.to_string(), id.to_string())
        })
        .collect()
}

/// The names in a directory, sorted.
fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}
