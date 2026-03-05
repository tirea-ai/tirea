"use client";

import { useEffect, useState } from "react";

export type StarterAgentId = "default" | "permission" | "stopper";

const SUPPORTED_AGENT_IDS: StarterAgentId[] = ["default", "permission", "stopper"];

export function normalizeStarterAgentId(value: string | null | undefined): StarterAgentId {
  if (!value) return "default";
  return SUPPORTED_AGENT_IDS.includes(value as StarterAgentId)
    ? (value as StarterAgentId)
    : "default";
}

export function useStarterAgentId(): StarterAgentId {
  const [agentId, setAgentId] = useState<StarterAgentId>("default");

  useEffect(() => {
    const query = new URLSearchParams(window.location.search);
    setAgentId(normalizeStarterAgentId(query.get("agentId")));
  }, []);

  return agentId;
}
