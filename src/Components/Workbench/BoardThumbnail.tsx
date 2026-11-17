import React, { useEffect, useState } from 'react';
import { readFile } from '@tauri-apps/plugin-fs';

interface BoardThumbnailProps {
  imagePath: string;
  marker: string;
  tone: string;
}

function imageMime(path: string) {
  const extension = path.split('.').pop()?.toLowerCase();
  if (extension === 'jpg' || extension === 'jpeg') return 'image/jpeg';
  if (extension === 'webp') return 'image/webp';
  return 'image/png';
}

export const BoardThumbnail: React.FC<BoardThumbnailProps> = ({ imagePath, marker, tone }) => {
  const [source, setSource] = useState('');

  useEffect(() => {
    let active = true;
    let objectUrl = '';
    setSource('');
    if (!imagePath) return () => undefined;
    void readFile(imagePath)
      .then((bytes) => {
        if (!active) return;
        objectUrl = URL.createObjectURL(new Blob([bytes], { type: imageMime(imagePath) }));
        setSource(objectUrl);
      })
      .catch(() => undefined);
    return () => {
      active = false;
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [imagePath]);

  return (
    <span className={`device-board-thumbnail is-${tone}`} title={imagePath || '未设置主板图片'}>
      {source ? (
        <img src={source} alt="主板实物" />
      ) : (
        <span className="device-board-thumbnail-placeholder" aria-label="主板图片未设置">
          <i />
          <i />
          <i />
          <b>{marker}</b>
          <em>PCB</em>
        </span>
      )}
    </span>
  );
};
