import { LeafletContext, createLeafletContext } from '@react-leaflet/core';
import { Map as LeafletMap } from 'leaflet';
import React, { forwardRef, useCallback, useEffect, useImperativeHandle, useRef, useState } from 'react';
function MapContainerComponent({ bounds, boundsOptions, center, children, className, id, placeholder, style, whenReady, zoom, ...options }, forwardedRef) {
    const [props] = useState({
        className,
        id,
        style
    });
    const [context, setContext] = useState(null);
    const mapInstanceRef = useRef(undefined);
    useImperativeHandle(forwardedRef, ()=>context?.map ?? null, [
        context
    ]);
    // biome-ignore lint/correctness/useExhaustiveDependencies: ref callback
    const mapRef = useCallback((node)=>{
        if (node !== null && !mapInstanceRef.current) {
            const map = new LeafletMap(node, options);
            mapInstanceRef.current = map;
            if (center != null && zoom != null) {
                map.setView(center, zoom);
            } else if (bounds != null) {
                map.fitBounds(bounds, boundsOptions);
            }
            if (whenReady != null) {
                map.whenReady(whenReady);
            }
            setContext(createLeafletContext(map));
        }
    }, []);
    useEffect(()=>{
        return ()=>{
            context?.map.remove();
        };
    }, [
        context
    ]);
    const contents = context ? /*#__PURE__*/ React.createElement(LeafletContext, {
        value: context
    }, children) : placeholder ?? null;
    return /*#__PURE__*/ React.createElement("div", {
        ...props,
        ref: mapRef
    }, contents);
}
export const MapContainer = /*#__PURE__*/ forwardRef(MapContainerComponent);
