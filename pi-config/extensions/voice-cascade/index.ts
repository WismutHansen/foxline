import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Type, type Static } from "typebox";
import { readFileSync } from "node:fs";

/**
 * voice-cascade — foxline "channels" spike (fxl-exmc, epic fxl-xwd9).
 *
 * Bitter-lesson factorization: this extension is a DUMB, DETERMINISTIC RUNNER.
 * All judgment lives in data:
 *   - the channel topology (which tiers exist, endpoints, models, budgets)
 *   - every prompt / steer template (authored per channel, swappable as models improve)
 * The code only: fetch with timeout, truncate deterministically, degrade silently.
 * If a tier stops earning its place under better models, delete its row in the
 * channel definition — no code changes.
 *
 * Channel definition resolution:
 *   1. $FOXLINE_CHANNEL_FILE (JSON), else
 *   2. built-in DEFAULT_CHANNEL below — a disposable example, not policy.
 */

interface TierDef {
	url: string;
	model: string;
	maxTokens?: number;
	timeoutMs?: number;
	extra?: Record<string, unknown>;
}

interface ChannelDef {
	enabled: boolean;
	draft: TierDef & {
		system: string;
		maxSentences: number;
		maxInputChars: number;
	};
	/** ${opening} is substituted with the drafted opening. */
	steerTemplate: string;
	orchestrator: {
		tier: TierDef;
		system: string;
	};
}

const DEFAULT_CHANNEL: ChannelDef = {
	enabled: true,
	draft: {
		url: process.env.FOXLINE_DRAFTER_URL ?? "http://localhost:11434",
		model: process.env.FOXLINE_DRAFTER_MODEL ?? "smollm2:135m",
		maxTokens: Number(process.env.FOXLINE_DRAFT_MAX_TOKENS ?? 60),
		timeoutMs: Number(process.env.FOXLINE_DRAFT_TIMEOUT_MS ?? 1500),
		maxSentences: Number(process.env.FOXLINE_DRAFT_MAX_SENTENCES ?? 2),
		maxInputChars: Number(process.env.FOXLINE_DRAFT_MAX_INPUT_CHARS ?? 2000),
		system:
			"You write the opening sentences of a friendly spoken voice assistant's reply. " +
			"Output ONLY those opening sentences: natural, short, spoken style, no markdown, no lists, no emoji.",
	},
	steerTemplate:
		"\n\n[voice-cascade] The fast tier already drafted this opening for your spoken reply: \"${opening}\"\n" +
		"Continue seamlessly from where it ends — same conversational spoken style, no markdown, do not repeat it verbatim. " +
		"If the draft is wrong or misses the point, silently correct course instead of following it.",
	orchestrator: {
		tier: {
			url: process.env.FOXLINE_ORCHESTRATOR_URL ?? "http://localhost:8000",
			model: process.env.FOXLINE_ORCHESTRATOR_MODEL ?? "deepseek-v4-flash",
			maxTokens: Number(process.env.FOXLINE_ORCHESTRATOR_MAX_TOKENS ?? 400),
			timeoutMs: Number(process.env.FOXLINE_ORCHESTRATOR_TIMEOUT_MS ?? 120_000),
			extra: { reasoning_effort: "low" },
		},
		system:
			"You are the behind-the-scenes reasoning engine of a spoken voice assistant. " +
			"Answer correctly and concisely in plain spoken prose: no markdown, no lists, no emoji, at most a few sentences.",
	},
};

function loadChannel(): ChannelDef {
	const file = process.env.FOXLINE_CHANNEL_FILE;
	if (!file) return DEFAULT_CHANNEL;
	try {
		const parsed = JSON.parse(readFileSync(file, "utf8")) as Partial<ChannelDef>;
		return {
			enabled: parsed.enabled ?? DEFAULT_CHANNEL.enabled,
			draft: { ...DEFAULT_CHANNEL.draft, ...(parsed.draft ?? {}) },
			steerTemplate: parsed.steerTemplate ?? DEFAULT_CHANNEL.steerTemplate,
			orchestrator: {
				tier: { ...DEFAULT_CHANNEL.orchestrator.tier, ...(parsed.orchestrator?.tier ?? {}) },
				system: parsed.orchestrator?.system ?? DEFAULT_CHANNEL.orchestrator.system,
			},
		};
	} catch (error) {
		// Fail loud: a broken channel file must not silently become the default.
		throw new Error(`[voice-cascade] cannot parse channel file ${file}: ${error}`);
	}
}

const CHANNEL = loadChannel();
const DEBUG = process.env.FOXLINE_CASCADE_DEBUG === "1";

async function chatCompletion(tier: TierDef, system: string, user: string): Promise<string | null> {
	const controller = new AbortController();
	const timer = setTimeout(() => controller.abort(), tier.timeoutMs ?? 30_000);
	try {
		const res = await fetch(`${tier.url}/v1/chat/completions`, {
			method: "POST",
			signal: controller.signal,
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({
				model: tier.model,
				messages: [
					{ role: "system", content: system },
					{ role: "user", content: user },
				],
				max_tokens: tier.maxTokens ?? 256,
				stream: false,
				...(tier.extra ?? {}),
			}),
		});
		if (!res.ok) return null;
		const data = (await res.json()) as { choices?: { message?: { content?: string | null } }[] };
		const text = data.choices?.[0]?.message?.content?.trim();
		return text && text.length > 0 ? text : null;
	} catch {
		return null;
	} finally {
		clearTimeout(timer);
	}
}

/** Deterministic normalization: take the first N sentences, hard-capped. */
function firstSentences(text: string, n: number): string {
	const cleaned = text.replace(/\s+/g, " ").trim();
	const parts = cleaned.match(/[^.!?]+[.!?]/g) ?? [cleaned];
	return parts.slice(0, Math.max(1, n)).join(" ").trim().slice(0, 400);
}

const OrchestratorParamsSchema = Type.Object({
	question: Type.String({ description: "The question or task needing deeper reasoning." }),
	context: Type.Optional(
		Type.String({ description: "Short spoken context of the ongoing conversation, if useful." }),
	),
});

type OrchestratorParams = Static<typeof OrchestratorParamsSchema>;

export default function voiceCascadeExtension(pi: ExtensionAPI) {
	if (!CHANNEL.enabled) return;

	// Drafter tier: rewrite the prompt so the speech model continues from an
	// instant opening. Failure is silent — the main brain just answers alone.
	pi.on("input", async (event) => {
		if (event.source === "extension") return { action: "continue" };
		const text = event.text.trim();
		if (!text || text.startsWith("/")) return { action: "continue" };

		const { url, model, system, maxTokens, timeoutMs, maxSentences, maxInputChars } = CHANNEL.draft;
		const input = text.length > maxInputChars ? text.slice(0, maxInputChars) : text;
		const raw = await chatCompletion({ url, model, maxTokens, timeoutMs }, system, input);
		if (!raw) {
			if (DEBUG) console.error("[voice-cascade] drafter failed/timeout — passing through");
			return { action: "continue" };
		}

		const opening = firstSentences(raw, maxSentences);
		if (DEBUG) console.error(`[voice-cascade] draft: ${opening}`);
		return {
			action: "transform",
			text: text + CHANNEL.steerTemplate.replace("${opening}", opening),
		};
	});

	// Orchestrator tier: hard queries go where the channel points. Whether to
	// escalate is the speech model's judgment (tool call), never a code rule.
	pi.registerTool({
		name: "ask_orchestrator",
		description:
			"Escalate a question to the deep-reasoning orchestrator model. Use for math, multi-step reasoning, facts you are unsure about, or planning. Returns a concise spoken-style answer; takes several seconds.",
		parameters: OrchestratorParamsSchema,
		async execute(_id: string, params: OrchestratorParams) {
			const user = params.context
				? `Conversation context (spoken, may be imperfect): ${params.context}\n\nQuestion: ${params.question}`
				: params.question;
			const answer = await chatCompletion(CHANNEL.orchestrator.tier, CHANNEL.orchestrator.system, user);
			if (!answer) {
				return {
					output: "The orchestrator is unavailable right now. Answer on your own as best you can.",
					isError: true,
				};
			}
			return { output: answer, isError: false };
		},
	});
}
