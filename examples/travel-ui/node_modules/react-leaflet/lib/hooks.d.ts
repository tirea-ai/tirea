import type { LeafletEventHandlerFnMap, Map as LeafletMap } from 'leaflet';
export declare function useMap(): LeafletMap;
export declare function useMapEvent<T extends keyof LeafletEventHandlerFnMap>(type: T, handler: LeafletEventHandlerFnMap[T]): LeafletMap;
export declare function useMapEvents(handlers: LeafletEventHandlerFnMap): LeafletMap;
