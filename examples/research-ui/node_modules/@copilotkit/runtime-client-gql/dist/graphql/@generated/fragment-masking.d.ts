import { DocumentTypeDecoration, ResultOf, TypedDocumentNode } from '@graphql-typed-document-node/core';
import { Incremental } from './graphql.js';

type FragmentType<TDocumentType extends DocumentTypeDecoration<any, any>> = TDocumentType extends DocumentTypeDecoration<infer TType, any> ? [TType] extends [{
    ' $fragmentName'?: infer TKey;
}] ? TKey extends string ? {
    ' $fragmentRefs'?: {
        [key in TKey]: TType;
    };
} : never : never : never;
declare function useFragment<TType>(_documentNode: DocumentTypeDecoration<TType, any>, fragmentType: FragmentType<DocumentTypeDecoration<TType, any>>): TType;
declare function useFragment<TType>(_documentNode: DocumentTypeDecoration<TType, any>, fragmentType: FragmentType<DocumentTypeDecoration<TType, any>> | undefined): TType | undefined;
declare function useFragment<TType>(_documentNode: DocumentTypeDecoration<TType, any>, fragmentType: FragmentType<DocumentTypeDecoration<TType, any>> | null): TType | null;
declare function useFragment<TType>(_documentNode: DocumentTypeDecoration<TType, any>, fragmentType: FragmentType<DocumentTypeDecoration<TType, any>> | null | undefined): TType | null | undefined;
declare function useFragment<TType>(_documentNode: DocumentTypeDecoration<TType, any>, fragmentType: Array<FragmentType<DocumentTypeDecoration<TType, any>>>): Array<TType>;
declare function useFragment<TType>(_documentNode: DocumentTypeDecoration<TType, any>, fragmentType: Array<FragmentType<DocumentTypeDecoration<TType, any>>> | null | undefined): Array<TType> | null | undefined;
declare function useFragment<TType>(_documentNode: DocumentTypeDecoration<TType, any>, fragmentType: ReadonlyArray<FragmentType<DocumentTypeDecoration<TType, any>>>): ReadonlyArray<TType>;
declare function useFragment<TType>(_documentNode: DocumentTypeDecoration<TType, any>, fragmentType: ReadonlyArray<FragmentType<DocumentTypeDecoration<TType, any>>> | null | undefined): ReadonlyArray<TType> | null | undefined;
declare function makeFragmentData<F extends DocumentTypeDecoration<any, any>, FT extends ResultOf<F>>(data: FT, _fragment: F): FragmentType<F>;
declare function isFragmentReady<TQuery, TFrag>(queryNode: DocumentTypeDecoration<TQuery, any>, fragmentNode: TypedDocumentNode<TFrag>, data: FragmentType<TypedDocumentNode<Incremental<TFrag>, any>> | null | undefined): data is FragmentType<typeof fragmentNode>;

export { FragmentType, isFragmentReady, makeFragmentData, useFragment };
