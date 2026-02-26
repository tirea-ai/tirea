import * as _graphql_typed_document_node_core from '@graphql-typed-document-node/core';
import { AvailableAgentsQuery, Exact, LoadAgentStateQuery, LoadAgentStateInput } from '../@generated/graphql.js';

declare const getAvailableAgentsQuery: _graphql_typed_document_node_core.TypedDocumentNode<AvailableAgentsQuery, Exact<{
    [key: string]: never;
}>>;
declare const loadAgentStateQuery: _graphql_typed_document_node_core.TypedDocumentNode<LoadAgentStateQuery, Exact<{
    data: LoadAgentStateInput;
}>>;

export { getAvailableAgentsQuery, loadAgentStateQuery };
