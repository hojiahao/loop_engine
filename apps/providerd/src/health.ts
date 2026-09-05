import { z } from "zod";

export const providerHealthSchema = z.object({
  component: z.literal("providerd"),
  protocolVersion: z.literal("loop-engine.v1alpha1"),
  status: z.literal("ready"),
});

export type ProviderHealth = z.infer<typeof providerHealthSchema>;

export function providerHealth(): ProviderHealth {
  return providerHealthSchema.parse({
    component: "providerd",
    protocolVersion: "loop-engine.v1alpha1",
    status: "ready",
  });
}
