import React from 'react';
import { Workbench, type WorkbenchProps } from './Workbench';

type WorkbenchPageProps = Omit<WorkbenchProps, 'isActive'> & { isActive?: boolean };

/** Top-level page for remote build, transfer, flash, and device validation. */
export const WorkbenchPage: React.FC<WorkbenchPageProps> = ({ isActive = true, ...actions }) => {
  return (
    <div className="device-workbench-page">
      <Workbench isActive={isActive} {...actions} />
    </div>
  );
};
