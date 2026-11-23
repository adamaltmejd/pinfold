// The proxy's facts, written into pi's system prompt at every start. The
// home seed also has the refusal line, but an existing home's or a
// project's own APPEND_SYSTEM.md can replace it. The proxy enforces both.
export default function operatingContext(pi) {
    pi.on("before_agent_start", (event) => {
        const allow = (process.env.PINFOLD_ALLOW ?? "")
            .split(/[\s,]+/)
            .filter(Boolean);
        event.systemPromptOptions.sections.pinfold = [
            `The proxy allows only these hosts: ${allow.length > 0 ? allow.join(", ") : "none"}.`,
            "A response carrying `Proxy-Status: pinfold` (usually a 403) is pinfold's sandbox refusing the request. It is final: do not retry it or route around it. `curl -v` shows the header and its reason.",
        ].join("\n");
    });
}
