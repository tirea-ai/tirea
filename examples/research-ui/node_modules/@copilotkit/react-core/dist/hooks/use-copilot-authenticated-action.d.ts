import { Parameter } from '@copilotkit/shared';
import { FrontendAction } from '../types/frontend-action.js';
import '@copilotkit/runtime-client-gql';
import 'react';

/**
 * Hook to create an authenticated action that requires user sign-in before execution.
 *
 * @remarks
 * This feature is only available when using CopilotKit's hosted cloud service.
 * To use this feature, sign up at https://cloud.copilotkit.ai to get your publicApiKey.
 *
 * @param action - The frontend action to be wrapped with authentication
 * @param dependencies - Optional array of dependencies that will trigger recreation of the action when changed
 */
declare function useCopilotAuthenticatedAction_c<T extends Parameter[]>(action: FrontendAction<T>, dependencies?: any[]): void;

export { useCopilotAuthenticatedAction_c };
