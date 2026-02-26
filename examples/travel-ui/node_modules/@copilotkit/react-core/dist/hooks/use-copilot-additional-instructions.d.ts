/**
 * Options for the useCopilotAdditionalInstructions hook.
 */
interface UseCopilotAdditionalInstructionsOptions {
    /**
     * The instructions to be added to the Copilot. Will be added to the instructions like so:
     *
     * ```txt
     * You are a helpful assistant.
     * Additionally, follow these instructions:
     * - Do not answer questions about the weather.
     * - Do not answer questions about the stock market.
     * ```
     */
    instructions: string;
    /**
     * Whether the instructions are available to the Copilot.
     */
    available?: "enabled" | "disabled";
}
/**
 * Adds the given instructions to the Copilot context.
 */
declare function useCopilotAdditionalInstructions({ instructions, available }: UseCopilotAdditionalInstructionsOptions, dependencies?: any[]): void;

export { UseCopilotAdditionalInstructionsOptions, useCopilotAdditionalInstructions };
