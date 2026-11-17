import React from 'react';
import { FontAwesomeIcon } from '@fortawesome/react-fontawesome';
import { faPaintBrush, faFont, faImage, faFilm, faPencil } from '@fortawesome/free-solid-svg-icons';
import { TextTab } from './tabs/TextTab';
import { ImageTab } from './tabs/ImageTab';
import { VideoTab } from './tabs/VideoTab';
import { DrawTab } from './tabs/DrawTab';
import type { TabContext, ScreenDisplayType } from './tabs/common';

export type ContentTab = 'text' | 'image' | 'video' | 'draw';

interface MiddlePanelProps extends TabContext {
  activeTab: ContentTab;
  onTabChange: (t: ContentTab) => void;
  /** 当前显示类型：rgb565 时显示"全刷/局刷"切换 */
  displayType: ScreenDisplayType;
  /** 推送范围：full=整屏，partial=只推内容外接矩形 */
  refreshMode: 'full' | 'partial';
  onRefreshModeChange: (m: 'full' | 'partial') => void;
}

const TABS: Array<{ key: ContentTab; label: string; icon: typeof faFont }> = [
  { key: 'text', label: '文本', icon: faFont },
  { key: 'image', label: '图片', icon: faImage },
  { key: 'video', label: '视频', icon: faFilm },
  { key: 'draw', label: '绘制', icon: faPencil },
];

/**
 * 中间面板：胶囊式 Tab 切换 + 4 个 Tab 内容（文本 / 图片 / 视频 / 绘制）。
 */
export const MiddlePanel: React.FC<MiddlePanelProps> = ({
  activeTab,
  onTabChange,
  width,
  height,
  onSend,
  busy,
  displayType,
  refreshMode,
  onRefreshModeChange,
}) => {
  const tabCtx: TabContext = { width, height, onSend, busy };

  return (
    <div className="sdt-panel">
      <div className="sdt-section-title">
        <FontAwesomeIcon icon={faPaintBrush} />
        <span>显示内容</span>
        {displayType === 'rgb565' && (
          <div className="sdt-refresh-toggle">
            <button
              className={`sdt-refresh-btn ${refreshMode === 'partial' ? 'active' : ''}`}
              onClick={() => onRefreshModeChange('partial')}
              title="只推内容所在的小矩形（快，几KB）"
            >
              局部刷新
            </button>
            <button
              className={`sdt-refresh-btn ${refreshMode === 'full' ? 'active' : ''}`}
              onClick={() => onRefreshModeChange('full')}
              title="推整屏 307KB（当前接口约100秒）"
            >
              全屏刷新
            </button>
          </div>
        )}
      </div>

      <div className="sdt-tab-bar">
        {TABS.map((t) => (
          <button
            key={t.key}
            className={`sdt-tab-btn ${activeTab === t.key ? 'active' : ''}`}
            onClick={() => onTabChange(t.key)}
          >
            <FontAwesomeIcon icon={t.icon} /> {t.label}
          </button>
        ))}
      </div>

      <div className="sdt-tab-content">
        {/* 用 conditional render 而不是 display:none —— 切走的 Tab 卸载，
            避免视频 Tab 后台继续占内存 */}
        {activeTab === 'text' && <TextTab {...tabCtx} />}
        {activeTab === 'image' && <ImageTab {...tabCtx} />}
        {activeTab === 'video' && <VideoTab {...tabCtx} />}
        {activeTab === 'draw' && <DrawTab {...tabCtx} />}
      </div>
    </div>
  );
};
