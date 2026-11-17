import React from 'react';
import { MCUWorkbench } from './MCUWorkbench';

interface MCUWorkbenchPageProps {
  isActive?: boolean;
}

export const MCUWorkbenchPage: React.FC<MCUWorkbenchPageProps> = ({ isActive = true }) => (
  <div className="device-workbench-page">
    <MCUWorkbench isActive={isActive} />
  </div>
);
