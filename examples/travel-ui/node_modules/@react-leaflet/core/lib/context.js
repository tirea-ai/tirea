import { createContext, use } from 'react';
export const CONTEXT_VERSION = 1;
export function createLeafletContext(map) {
    return Object.freeze({
        __version: CONTEXT_VERSION,
        map
    });
}
export function extendContext(source, extra) {
    return Object.freeze({
        ...source,
        ...extra
    });
}
export const LeafletContext = createContext(null);
export function useLeafletContext() {
    const context = use(LeafletContext);
    if (context == null) {
        throw new Error('No context provided: useLeafletContext() can only be used in a descendant of <MapContainer>');
    }
    return context;
}
