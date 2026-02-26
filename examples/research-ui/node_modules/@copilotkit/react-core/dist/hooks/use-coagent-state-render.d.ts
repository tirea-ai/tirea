import { CoAgentStateRender } from '../types/coagent-action.js';

/**
 * The useCoAgentStateRender hook allows you to render UI or text based components on a Agentic Copilot's state in the chat.
 * This is particularly useful for showing intermediate state or progress during Agentic Copilot operations.
 *
 * ## Usage
 *
 * ### Simple Usage
 *
 * ```tsx
 * import { useCoAgentStateRender } from "@copilotkit/react-core";
 *
 * type YourAgentState = {
 *   agent_state_property: string;
 * }
 *
 * useCoAgentStateRender<YourAgentState>({
 *   name: "basic_agent",
 *   nodeName: "optionally_specify_a_specific_node",
 *   render: ({ status, state, nodeName }) => {
 *     return (
 *       <YourComponent
 *         agentStateProperty={state.agent_state_property}
 *         status={status}
 *         nodeName={nodeName}
 *       />
 *     );
 *   },
 * });
 * ```
 *
 * This allows for you to render UI components or text based on what is happening within the agent.
 *
 * ### Example
 * A great example of this is in our Perplexity Clone where we render the progress of an agent's internet search as it is happening.
 * You can play around with it below or learn how to build it with its [demo](/coagents/videos/perplexity-clone).
 *
 * <Callout type="info">
 *   This example is hosted on Vercel and may take a few seconds to load.
 * </Callout>
 *
 * <iframe src="https://examples-coagents-ai-researcher-ui.vercel.app/" className="w-full rounded-lg border h-[700px] my-4" />
 */

/**
 * This hook is used to render agent state with custom UI components or text. This is particularly
 * useful for showing intermediate state or progress during Agentic Copilot operations.
 * To get started using rendering intermediate state through this hook, checkout the documentation.
 *
 * https://docs.copilotkit.ai/coagents/shared-state/predictive-state-updates
 */
declare function useCoAgentStateRender<T = any>(action: CoAgentStateRender<T>, dependencies?: any[]): void;

export { useCoAgentStateRender };
