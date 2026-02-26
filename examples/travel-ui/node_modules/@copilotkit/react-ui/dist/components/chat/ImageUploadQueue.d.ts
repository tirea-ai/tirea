import React__default from 'react';

interface ImageUploadQueueProps {
    images: Array<{
        contentType: string;
        bytes: string;
    }>;
    onRemoveImage: (index: number) => void;
    className?: string;
}
declare const ImageUploadQueue: React__default.FC<ImageUploadQueueProps>;

export { ImageUploadQueue };
