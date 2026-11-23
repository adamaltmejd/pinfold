// The proxy's allowlist, written into pi's system prompt at each start so
// the model does not spend turns finding it. The proxy enforces it.
export default function operatingContext(pi) {
    pi.on("before_agent_start", (event) => {
        const allow = (process.env.PINFOLD_ALLOW ?? "")
            .split(/[\s,]+/)
            .filter(Boolean);
        event.systemPromptOptions.sections.pinfold =
            `The proxy allows only these hosts: ${allow.length > 0 ? allow.join(", ") : "none"}.`;
    });
}
