import React from 'react';
import { AISupervisor, type AISupervisorSection } from './AISupervisor';

export const AISupervisorPage: React.FC<{ section: AISupervisorSection; isActive: boolean }> = (
  props
) => <AISupervisor {...props} />;

export default AISupervisorPage;
