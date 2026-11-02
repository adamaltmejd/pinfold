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
            "- In a Git project, commit your work here; the user runs `pinfold fetch` to bring branches back. Outside Git, save deliverables in /work. The user can explicitly export retained disk files. Nothing is copied to the host automatically.",
        ].join("\n");
        event.systemPromptOptions.sections.pinfold = facts;
    });
}
