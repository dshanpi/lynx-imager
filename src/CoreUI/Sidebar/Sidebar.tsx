import React, { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { motion } from 'framer-motion';
import { useAppVersion } from '../../Hooks/useAppVersion';
import { LynxIcon, type LynxIconName } from '../LynxIcon';
import './Sidebar.css';

/**
 * Tool item configuration for sidebar navigation.
 *
 * Defines a single tool entry in the sidebar navigation,
 * including its identifier, display name, icon, and optional description.
 */
export interface ToolItem {
  /** Unique tool identifier */
  id: string;
  /** Display name shown in sidebar */
  name: string;
  /** Semantic icon rendered by the unified Fluent icon component. */
  icon: LynxIconName;
  /** Optional tooltip/description text */
  description?: string;
  /** Optional second-level navigation items. Group items do not select a page directly. */
  children?: ToolItem[];
}

/**
 * Sidebar component props.
 *
 * Configuration for the sidebar navigation panel including
 * tool list, active state, and interaction callbacks.
 */
interface SidebarProps {
  /** Available tool items for navigation */
  tools: ToolItem[];
  /** Currently active tool ID */
  activeTool: string;
  /** Callback when a tool is selected */
  onToolSelect: (toolId: string) => void;
  /** Whether sidebar is collapsed */
  collapsed: boolean;
  /** Callback to toggle collapse state */
  onToggleCollapse: () => void;
  /** Optional callback for settings button */
  onSettingsClick?: () => void;
  /** Optional callback for login button */
  onLoginClick?: () => void;
  /** Optional logged-in user display name (null/undefined shows "登录") */
  authUserName?: string | null;
  /** Whether sidebar interactions are locked */
  locked?: boolean;
  /** Optional node rendered in a dedicated area between the nav and the
   *  login/settings footer (e.g. the global voltage toggle button). */
  controlArea?: React.ReactNode;
}

/**
 * Sidebar navigation component.
 *
 * Sidebar provides the main navigation interface for the application,
 * displaying a collapsible list of tool buttons with icons and names.
 * It includes an expand/collapse toggle, version display, and settings access.
 *
 * When collapsed, only icons are shown. When expanded, tool names and
 * application title are visible. The sidebar can be locked to prevent
 * navigation changes during operations like flashing.
 *
 * Features:
 * - Animated tool buttons with hover/tap effects
 * - Version number display from package.json
 * - Internationalized labels via react-i18next
 * - Disabled state when locked
 *
 * Example usage:
 * ```tsx
 * const tools: ToolItem[] = [
 *   { id: 'loader', name: 'Firmware Loader', icon: faDownload },
 *   { id: 'flasher', name: 'Device Flash', icon: faBolt },
 * ];
 * <Sidebar
 *   tools={tools}
 *   activeTool="loader"
 *   onToolSelect={setActiveTool}
 *   collapsed={isCollapsed}
 *   onToggleCollapse={toggleCollapse}
 * />
 * ```
 */
export const Sidebar: React.FC<SidebarProps> = ({
  tools,
  activeTool,
  onToolSelect,
  collapsed,
  onToggleCollapse,
  onSettingsClick,
  onLoginClick,
  authUserName,
  locked = false,
  controlArea,
}) => {
  const { t } = useTranslation();
  const displayVersion = useAppVersion();
  const [expandedGroups, setExpandedGroups] = useState<Record<string, boolean>>(() =>
    Object.fromEntries(tools.filter((tool) => tool.children?.length).map((tool) => [tool.id, true]))
  );

  useEffect(() => {
    const activeGroup = tools.find((tool) =>
      tool.children?.some((child) => child.id === activeTool)
    );
    if (activeGroup) {
      setExpandedGroups((current) =>
        current[activeGroup.id] === false ? { ...current, [activeGroup.id]: true } : current
      );
    }
  }, [activeTool, tools]);

  const renderToolButton = (tool: ToolItem, nested = false) => {
    const isActive = activeTool === tool.id;
    const isDisabled = locked && !isActive;
    return (
      <motion.button
        key={tool.id}
        data-testid={`sidebar-tool-${tool.id}`}
        className={`sidebar-item ${nested ? 'sidebar-subitem' : ''} ${isActive ? 'active' : ''} ${isDisabled ? 'sidebar-item--disabled' : ''}`}
        onClick={() => !isDisabled && onToolSelect(tool.id)}
        title={collapsed ? tool.name : tool.description}
        disabled={isDisabled}
      >
        <span className="sidebar-item-icon">
          <LynxIcon name={tool.icon} filled={isActive} size={20} />
        </span>
        {!collapsed && <span className="sidebar-item-name">{tool.name}</span>}
      </motion.button>
    );
  };

  return (
    <div
      data-testid="app-sidebar"
      className={`sidebar ${collapsed ? 'sidebar--collapsed' : ''} ${locked ? 'sidebar--locked' : ''}`}
    >
      <div className="sidebar-header">
        {!collapsed && (
          <div className="sidebar-brand">
            <h1>{t('sidebar.title')}</h1>
            <div className="sidebar-release">
              <span className="sidebar-version">v{displayVersion}</span>
              <i aria-hidden="true" />
              <span>{t('sidebar.releaseChannel')}</span>
            </div>
            <span className="sidebar-subtitle">{t('sidebar.subtitle')}</span>
          </div>
        )}
        <motion.button
          data-testid="sidebar-toggle"
          className="sidebar-toggle"
          onClick={onToggleCollapse}
          title={collapsed ? t('sidebar.expand') : t('sidebar.collapse')}
          disabled={locked}
        >
          <LynxIcon name={collapsed ? 'panel-expand' : 'panel-contract'} size={20} />
        </motion.button>
      </div>
      <nav className="sidebar-nav">
        {tools.map((tool) => {
          if (!tool.children?.length) return renderToolButton(tool);

          const isExpanded = expandedGroups[tool.id] !== false;
          const containsActiveTool = tool.children.some((child) => child.id === activeTool);
          return (
            <div className="sidebar-group" key={tool.id}>
              <motion.button
                data-testid={`sidebar-group-${tool.id}`}
                className={`sidebar-item sidebar-group-toggle ${containsActiveTool ? 'has-active-child' : ''}`}
                onClick={() =>
                  setExpandedGroups((current) => ({ ...current, [tool.id]: !isExpanded }))
                }
                title={collapsed ? tool.name : tool.description}
                disabled={locked}
                aria-expanded={isExpanded}
              >
                <span className="sidebar-item-icon">
                  <LynxIcon name={tool.icon} filled={containsActiveTool} size={20} />
                </span>
                {!collapsed && (
                  <>
                    <span className="sidebar-item-name">{tool.name}</span>
                    <LynxIcon
                      className="sidebar-group-chevron"
                      name={isExpanded ? 'chevron-down' : 'chevron-right'}
                      size={16}
                    />
                  </>
                )}
              </motion.button>
              {isExpanded && (
                <div className="sidebar-subnav">
                  {tool.children.map((child) => renderToolButton(child, true))}
                </div>
              )}
            </div>
          );
        })}
      </nav>
      {controlArea && <div className="sidebar-control-area">{controlArea}</div>}
      <div className="sidebar-footer">
        {onLoginClick && (
          <motion.button
            data-testid="sidebar-login"
            className="sidebar-footer-btn"
            onClick={onLoginClick}
            title={authUserName ? authUserName : t('sidebar.login')}
          >
            <LynxIcon name="person" filled={Boolean(authUserName)} size={20} />
            {!collapsed && <span>{authUserName ? authUserName : t('sidebar.login')}</span>}
          </motion.button>
        )}
        {onSettingsClick && (
          <motion.button
            data-testid="sidebar-settings"
            className="sidebar-footer-btn"
            onClick={onSettingsClick}
            title={t('sidebar.settings')}
            disabled={locked}
          >
            <LynxIcon name="settings" size={20} />
            {!collapsed && <span>{t('sidebar.settings')}</span>}
          </motion.button>
        )}
      </div>
    </div>
  );
};

export default Sidebar;
