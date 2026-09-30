//! Guarantee 30: launch names stale image inputs without refusing the run.

use std::fs;
use std::path::Path;

use e2e::{
    ImageCleanup, TestDir, TestEnv, build_profile, default_image, git, pinfold,
    profile_containerfile, project_id, run_ok,
};

#[test]
fn changed_image_inputs_prompt_a_rebuild() {
    // Sabotage: remove the profile Containerfile comparison in ensure_image;
    // changing the fixture's profile then produces no image-outdated hint.
    // Remove the project Containerfile comparison and its trusted change
    // produces no hint. Remove the recorded-base comparison and rebuilding
    // only the profile leaves the project silently stale. Warn unconditionally
    // and the unchanged/rebuilt controls fail. Expectations come from the
    // fixture's changed inputs and the spec's image-outdated token.
    let env = TestEnv::new("image-warning");
    let default = default_image(&env);
    let profile = format!("image-warning-{}", std::process::id());
    let _profile_images = ImageCleanup {
        repository: format!("pinfold/profile-{profile}"),
    };
    let profile_file = profile_containerfile(
        &env,
        &profile,
        &format!("FROM {default}\nENV IMAGE_WARNING_FIXTURE=first\n"),
    );
    build_profile(&env, &profile);
    let project = TestDir::new(&env, "project");
    git(project.path(), &["init", "-q"]);
    let config = project.path().join(".pinfold.toml");
    fs::write(&config, format!("profile = \"{profile}\"\n")).unwrap();
    allow(&env, project.path());
    version(&env, project.path(), false);

    fs::write(
        &profile_file,
        format!("FROM {default}\nENV IMAGE_WARNING_FIXTURE=second\n"),
    )
    .unwrap();
    version(&env, project.path(), true);
    build_profile(&env, &profile);
    version(&env, project.path(), false);

    // A project's own Containerfile adds a second input. Trust must be renewed
    // after changing it, so the trust refusal cannot stand in for this hint.
    let project_file = project.path().join("Containerfile.pinfold");
    let project_contents = format!("FROM pinfold/profile-{profile}:latest\n");
    fs::write(&project_file, &project_contents).unwrap();
    fs::write(
        &config,
        format!("profile = \"{profile}\"\ncontainerfile = \"Containerfile.pinfold\"\n"),
    )
    .unwrap();
    allow(&env, project.path());
    build(&env, project.path());
    let _project_images = ImageCleanup {
        repository: format!("pinfold/project-{}", project_id(&env, project.path())),
    };
    version(&env, project.path(), false);
    fs::write(
        &project_file,
        format!("{project_contents}ENV PROJECT_IMAGE_WARNING_FIXTURE=changed\n"),
    )
    .unwrap();
    allow(&env, project.path());
    version(&env, project.path(), true);
    build(&env, project.path());
    version(&env, project.path(), false);

    // Profile bytes can change underneath an unchanged project image before
    // its base digest changes. Rebuilding that profile changes the digest,
    // but the project remains stale until it too is rebuilt.
    fs::write(
        &profile_file,
        format!("FROM {default}\nENV IMAGE_WARNING_FIXTURE=third\n"),
    )
    .unwrap();
    version(&env, project.path(), true);
    build_profile(&env, &profile);
    version(&env, project.path(), true);
    build(&env, project.path());
    version(&env, project.path(), false);
}

fn allow(env: &TestEnv, project: &Path) {
    run_ok(env.command(pinfold()).arg("allow").current_dir(project));
}

fn build(env: &TestEnv, project: &Path) {
    run_ok(env.command(pinfold()).arg("build").current_dir(project));
}

#[track_caller]
fn version(env: &TestEnv, project: &Path, outdated: bool) {
    let output = run_ok(
        env.command(pinfold())
            .args(["pi", "--version"])
            .current_dir(project),
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        stderr.contains("image-outdated"),
        outdated,
        "unexpected image hint: {stderr}"
    );
}
