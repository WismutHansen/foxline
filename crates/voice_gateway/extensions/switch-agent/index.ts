import type { ExtensionAPI, ToolResult } from "@earendil-works/pi-coding-agent";
import { Type, type Static } from "typebox";
import net from "node:net";

const ParamsSchema = Type.Object({
  workspace: Type.String({
    description:
      "Target workspace id from the gateway's [workspaces.registry] (see the running gateway's config.toml).",
  }),
  persona: Type.Optional(
    Type.String({
      description: "Optional persona override; defaults to the target workspace's configured persona.",
    })
  ),
});

type Params = Static<typeof ParamsSchema>;

type SwitchAgentReply = {
  ok: boolean;
  workspace?: string;
  agent?: string;
  persona?: string;
  loadout?: string;
  error?: string;
};

const CONNECT_TIMEOUT_MS = Number(process.env.FOXLINE_CONTROL_CONNECT_TIMEOUT_MS ?? 2000);
const REPLY_TIMEOUT_MS = Number(process.env.FOXLINE_CONTROL_REPLY_TIMEOUT_MS ?? 31000);

// Opt-in only: reference this as `builtin:switch-agent` in a workspace's
// .foxline/loadout.toml `extensions = [...]` list. Unlike foxline-tools (which
// every agent loads via scripts/sync-pi-extensions.sh), this is Foxline-specific
// and must be requested per workspace.
export default function switchAgentExtension(pi: ExtensionAPI) {
  pi.registerTool({
    name: "switch_agent",
    label: "Switch Agent",
    description:
      "Switches the live Foxline voice session to a different workspace/agent (and optionally persona) " +
      "without ending the session. The target must be a workspace id configured in the gateway's " +
      "[workspaces.registry], not an arbitrary path.",
    parameters: ParamsSchema,
    async execute(_toolCallId, params: Params): Promise<ToolResult> {
      const addr = process.env.FOXLINE_CONTROL_ADDR;
      const token = process.env.FOXLINE_CONTROL_TOKEN;
      if (!addr || !token) {
        return errorResult(
          "switch_agent is unavailable: this session was not launched with a Foxline control channel"
        );
      }
      const [host, port] = splitAddr(addr);
      if (!host || !Number.isFinite(port)) {
        return errorResult(`switch_agent has a malformed FOXLINE_CONTROL_ADDR: ${addr}`);
      }

      try {
        const reply = await sendSwitchRequest(host, port, {
          token,
          workspace: params.workspace,
          persona: params.persona,
        });
        if (!reply.ok) {
          return errorResult(reply.error ?? "switch_agent failed");
        }
        return okResult(reply);
      } catch (error) {
        return errorResult(error instanceof Error ? error.message : String(error));
      }
    },
  });
}

function splitAddr(addr: string): [string, number] {
  const idx = addr.lastIndexOf(":");
  if (idx === -1) return ["", NaN];
  return [addr.slice(0, idx), Number(addr.slice(idx + 1))];
}

// Raw TCP JSON-lines request/response: the gateway's control listener
// (crates/voice_gateway/src/control.rs) accepts one connection, reads one
// request line, writes one reply line, then closes.
function sendSwitchRequest(
  host: string,
  port: number,
  request: { token: string; workspace: string; persona?: string }
): Promise<SwitchAgentReply> {
  return new Promise((resolve, reject) => {
    const socket = net.createConnection({ host, port });
    let buffer = "";
    let settled = false;

    const finish = (fn: () => void) => {
      if (settled) return;
      settled = true;
      clearTimeout(connectTimer);
      clearTimeout(replyTimer);
      socket.destroy();
      fn();
    };

    const connectTimer = setTimeout(
      () => finish(() => reject(new Error("switch_agent control connection timed out"))),
      CONNECT_TIMEOUT_MS
    );
    const replyTimer = setTimeout(
      () => finish(() => reject(new Error("switch_agent timed out waiting for a reply"))),
      REPLY_TIMEOUT_MS
    );

    socket.once("connect", () => {
      clearTimeout(connectTimer);
      socket.write(`${JSON.stringify(request)}\n`);
    });

    socket.on("data", (chunk) => {
      buffer += chunk.toString("utf8");
      const newlineIndex = buffer.indexOf("\n");
      if (newlineIndex === -1) return;
      const line = buffer.slice(0, newlineIndex);
      finish(() => {
        try {
          resolve(JSON.parse(line) as SwitchAgentReply);
        } catch (error) {
          reject(error instanceof Error ? error : new Error(String(error)));
        }
      });
    });

    socket.once("error", (error) => finish(() => reject(error)));
    socket.once("close", () =>
      finish(() => reject(new Error("switch_agent control connection closed before a reply arrived")))
    );
  });
}

function okResult(details: SwitchAgentReply): ToolResult {
  const summary = `Switched to workspace "${details.workspace}" (agent: ${details.agent}, persona: ${details.persona}, loadout: ${details.loadout}).`;
  return { content: [{ type: "text", text: summary }], details };
}

function errorResult(message: string): ToolResult {
  return {
    content: [{ type: "text", text: message }],
    isError: true,
    details: { ok: false, error: message },
  };
}
