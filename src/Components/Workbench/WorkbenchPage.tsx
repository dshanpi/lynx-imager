import React from 'react';
import { Workbench } from './Workbench';

interface WorkbenchPageProps {
  isActive?: boolean;
}

/** Top-level page for remote build, transfer, flash, and device validation. */
export const WorkbenchPage: React.FC<WorkbenchPageProps> = ({ isActive = true }) => {
  return (
    <div className="device-workbench-page">
      <Workbench isActive={isActive} />
    </div>
  );
};
