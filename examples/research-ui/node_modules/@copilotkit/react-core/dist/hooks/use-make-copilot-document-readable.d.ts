import { DocumentPointer } from '../types/document-pointer.js';

/**
 * Makes a document readable by Copilot.
 * @param document The document to make readable.
 * @param categories The categories to associate with the document.
 * @param dependencies The dependencies to use for the effect.
 * @returns The id of the document.
 */
declare function useMakeCopilotDocumentReadable(document: DocumentPointer, categories?: string[], dependencies?: any[]): string | undefined;

export { useMakeCopilotDocumentReadable };
