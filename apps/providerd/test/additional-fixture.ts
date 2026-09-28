import type { Deployment } from "../src/config.js";
import { validate_deployment } from "../src/config.js";
import { rich_fixture } from "./rich-fixture.js";

export const ADDITIONAL = ["google_generate", "google_interactions", "cohere"] as const;
export type Additional = (typeof ADDITIONAL)[number];

export function thinking_config(config: Deployment): void {
  for (const model of config.models) {
    if (model.plugin.startsWith("google")) model.reasoning = "low";
    else {
      model.reasoning = "enabled";
      model.thinking_tokens = 1024;
      model.output_tokens = 2048;
    }
  }
  config.policy.output_tokens = 2048;
}

export async function additional_fixture(
  directory: string,
  configure?: (config: Deployment) => void,
) {
  return rich_fixture(directory, (config) => {
    const base = config.models[0];
    if (!base) throw new Error("fixture_model_missing");
    config.models = ADDITIONAL.map((plugin) => ({
      ...base,
      id: plugin,
      alias: plugin,
      plugin,
      model: `${plugin}-fixture-20260901`,
      ...(plugin !== "google_generate" ? { input_token_limit: 128 } : {}),
      features: { ...base.features, documents: plugin !== "cohere" },
    }));
    const principal = config.principals[0];
    if (!principal) throw new Error("fixture_principal_missing");
    principal.model_ids = [...ADDITIONAL];
    configure?.(config);
    validate_deployment(config);
  });
}

export function additional_reply(plugin: Additional, tools = false, reasoning = false) {
  if (plugin === "cohere")
    return {
      id: "cohere-fixture",
      finish_reason: tools ? "TOOL_CALL" : "COMPLETE",
      message: {
        role: "assistant",
        ...(tools
          ? {
              tool_plan: "Plan",
              tool_calls: [
                {
                  type: "function",
                  id: "call_1",
                  function: { name: "propose_factor", arguments: '{"window":20}' },
                },
              ],
            }
          : { content: [{ type: "text", text: "diagnostic idea" }] }),
      },
      usage: {
        tokens: { input_tokens: 12, output_tokens: 5 },
        billed_units: { input_tokens: 5, output_tokens: 2 },
      },
    };
  if (plugin === "google_interactions")
    return {
      id: "interaction-fixture",
      object: "interaction",
      model: `${plugin}-fixture-20260901`,
      status: tools ? "requires_action" : "completed",
      steps: [
        ...(reasoning
          ? [
              {
                type: "thought",
                summary: [{ type: "text", text: "Summary" }],
                signature: "signed-thinking",
              },
            ]
          : []),
        tools
          ? {
              type: "function_call",
              id: "call_1",
              name: "propose_factor",
              arguments: { window: 20 },
            }
          : { type: "model_output", content: [{ type: "text", text: "diagnostic idea" }] },
      ],
      usage: {
        total_input_tokens: 12,
        total_output_tokens: 5,
        total_thought_tokens: reasoning ? 3 : 0,
        total_tokens: reasoning ? 20 : 17,
      },
    };
  return {
    responseId: "google-fixture",
    modelVersion: `${plugin}-fixture-20260901`,
    candidates: [
      {
        index: 0,
        finishReason: "STOP",
        content: {
          role: "model",
          parts: [
            ...(reasoning ? [{ thought: true, text: "Summary" }] : []),
            {
              ...(tools
                ? { functionCall: { name: "propose_factor", args: { window: 20 } } }
                : { text: "diagnostic idea" }),
              ...(reasoning ? { thoughtSignature: "signed-thinking" } : {}),
            },
          ],
        },
      },
    ],
    usageMetadata: {
      promptTokenCount: 12,
      candidatesTokenCount: 5,
      thoughtsTokenCount: reasoning ? 3 : 0,
      totalTokenCount: reasoning ? 20 : 17,
    },
  };
}

export function additional_events(plugin: Additional, tools = false): unknown[] {
  if (plugin === "cohere")
    return [
      { type: "message-start", id: "c1", delta: { message: { role: "assistant" } } },
      ...(tools
        ? [
            { type: "tool-plan-delta", delta: { message: { tool_plan: "Plan" } } },
            {
              type: "tool-call-start",
              index: 0,
              delta: {
                message: {
                  tool_calls: {
                    id: "call_1",
                    type: "function",
                    function: { name: "propose_factor", arguments: "" },
                  },
                },
              },
            },
            {
              type: "tool-call-delta",
              index: 0,
              delta: { message: { tool_calls: { function: { arguments: '{"window":' } } } },
            },
            {
              type: "tool-call-delta",
              index: 0,
              delta: { message: { tool_calls: { function: { arguments: "20}" } } } },
            },
            { type: "tool-call-end", index: 0 },
          ]
        : [
            {
              type: "content-start",
              index: 0,
              delta: { message: { content: { type: "text", text: "" } } },
            },
            {
              type: "content-delta",
              index: 0,
              delta: { message: { content: { text: "diagnostic " } } },
            },
            { type: "content-delta", index: 0, delta: { message: { content: { text: "idea" } } } },
            { type: "content-end", index: 0 },
          ]),
      {
        type: "message-end",
        delta: {
          finish_reason: tools ? "TOOL_CALL" : "COMPLETE",
          usage: { tokens: { input_tokens: 12, output_tokens: 5 } },
        },
      },
    ];
  if (plugin === "google_interactions")
    return [
      {
        event_type: "interaction.created",
        interaction: { id: "i1", model: `${plugin}-fixture-20260901`, status: "in_progress" },
      },
      {
        event_type: "step.start",
        index: 0,
        step: tools
          ? { type: "function_call", id: "call_1", name: "propose_factor", arguments: {} }
          : { type: "model_output", content: [] },
      },
      {
        event_type: "step.delta",
        index: 0,
        delta: tools
          ? { type: "arguments_delta", arguments: '{"window":' }
          : { type: "text", text: "diagnostic " },
      },
      {
        event_type: "step.delta",
        index: 0,
        delta: tools
          ? { type: "arguments_delta", arguments: "20}" }
          : { type: "text", text: "idea" },
      },
      { event_type: "step.stop", index: 0 },
      {
        event_type: "interaction.completed",
        interaction: {
          id: "i1",
          status: tools ? "requires_action" : "completed",
          usage: { total_input_tokens: 12, total_output_tokens: 5, total_tokens: 17 },
        },
      },
    ];
  return [
    {
      responseId: "g1",
      modelVersion: `${plugin}-fixture-20260901`,
      candidates: [
        {
          index: 0,
          content: {
            role: "model",
            parts: tools
              ? [{ functionCall: { id: "call_1", name: "propose_factor", args: { window: 20 } } }]
              : [{ text: "diagnostic " }],
          },
        },
      ],
    },
    {
      responseId: "g1",
      modelVersion: `${plugin}-fixture-20260901`,
      candidates: [
        {
          index: 0,
          finishReason: "STOP",
          content: { role: "model", parts: tools ? [] : [{ text: "idea" }] },
        },
      ],
      usageMetadata: { promptTokenCount: 12, candidatesTokenCount: 5, totalTokenCount: 17 },
    },
  ];
}
