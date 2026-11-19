// The box's operating context, written into pi's system prompt at start.
// pinfold's proxy enforces the allowlist; this only states the facts so the
// model does not spend turns rediscovering them.
export default function operatingContext(pi) {
    pi.on("before_agent_start", (event) => {
        const allow = (process.env.PINFOLD_ALLOW ?? "")
            .split(/[\s,]+/)
            .filter(Boolean);
        const facts = [
            "## Box operating context",
            `- The proxy allows only these hosts: ${allow.length > 0 ? allow.join(", ") : "none"}.`,
            "- A proxy 403 is final. Do not retry it, route around it, or probe other hosts.",
            "- Work and save deliverables in the project’s actual current directory. In a share project, edits already affect host files, and your commits are the host's commits. In a disk project, commit work here; the user runs `git fetch box` to bring branches back, or explicitly exports other retained disk files. These instructions are guidance, not enforcement.",
        ].join("\n");
        event.systemPromptOptions.sections.pinfold = facts;
    });
}
