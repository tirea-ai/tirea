import { MessageInput, GenerateCopilotResponseMutation } from '../graphql/@generated/graphql.js';
import { Message } from './types.js';
import '@graphql-typed-document-node/core';

declare function filterAgentStateMessages(messages: Message[]): Message[];
declare function convertMessagesToGqlInput(messages: Message[]): MessageInput[];
declare function filterAdjacentAgentStateMessages(messages: GenerateCopilotResponseMutation["generateCopilotResponse"]["messages"]): GenerateCopilotResponseMutation["generateCopilotResponse"]["messages"];
declare function convertGqlOutputToMessages(messages: GenerateCopilotResponseMutation["generateCopilotResponse"]["messages"]): Message[];
declare function loadMessagesFromJsonRepresentation(json: any[]): Message[];

export { convertGqlOutputToMessages, convertMessagesToGqlInput, filterAdjacentAgentStateMessages, filterAgentStateMessages, loadMessagesFromJsonRepresentation };
