import * as _graphql_typed_document_node_core from '@graphql-typed-document-node/core';
import { GenerateCopilotResponseMutation, Exact, GenerateCopilotResponseInput, InputMaybe, Scalars } from '../@generated/graphql.js';

declare const generateCopilotResponseMutation: _graphql_typed_document_node_core.TypedDocumentNode<GenerateCopilotResponseMutation, Exact<{
    data: GenerateCopilotResponseInput;
    properties?: InputMaybe<Scalars["JSONObject"]["input"]>;
}>>;

export { generateCopilotResponseMutation };
