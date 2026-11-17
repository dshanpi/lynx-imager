import React from 'react';
import { HardwareMonitor } from './HardwareMonitor';

export const HardwareMonitorPage: React.FC<{ isActive: boolean }> = (props) => (
  <HardwareMonitor {...props} />
);
