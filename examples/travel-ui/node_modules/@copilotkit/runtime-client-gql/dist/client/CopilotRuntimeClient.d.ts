import * as urql from 'urql';
import { OperationResultSource, OperationResult } from 'urql';
import { GenerateCopilotResponseMutationVariables, GenerateCopilotResponseMutation, Exact, GenerateCopilotResponseInput, InputMaybe, Scalars, AvailableAgentsQuery, LoadAgentStateQuery } from '../graphql/@generated/graphql.js';
import { Client } from '@urql/core';
import '@graphql-typed-document-node/core';

interface CopilotRuntimeClientOptions {
    url: string;
    publicApiKey?: string;
    headers?: Record<string, string>;
    credentials?: RequestCredentials;
    handleGQLErrors?: (error: Error) => void;
    handleGQLWarning?: (warning: string) => void;
}
declare class CopilotRuntimeClient {
    client: Client;
    handleGQLErrors?: (error: Error) => void;
    handleGQLWarning?: (warning: string) => void;
    constructor(options: CopilotRuntimeClientOptions);
    generateCopilotResponse({ data, properties, signal, }: {
        data: GenerateCopilotResponseMutationVariables["data"];
        properties?: GenerateCopilotResponseMutationVariables["properties"];
        signal?: AbortSignal;
    }): OperationResultSource<OperationResult<GenerateCopilotResponseMutation, Exact<{
        data: GenerateCopilotResponseInput;
        properties?: InputMaybe<Scalars["JSONObject"]["input"]>;
    }>>>;
    asStream<S, T>(source: OperationResultSource<OperationResult<S, {
        data: T;
    }>>): ReadableStream<S>;
    availableAgents(): OperationResultSource<OperationResult<AvailableAgentsQuery, urql.AnyVariables>>;
    loadAgentState(data: {
        threadId: string;
        agentName: string;
    }): OperationResultSource<OperationResult<LoadAgentStateQuery, urql.AnyVariables>>;
    static removeGraphQLTypename(data: any): any;
}

export { CopilotRuntimeClient, CopilotRuntimeClientOptions };
