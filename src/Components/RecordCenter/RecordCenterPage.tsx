import React from 'react';
import { RecordCenter } from './RecordCenter';

export const RecordCenterPage: React.FC<{ isActive: boolean; actorId: string }> = (props) => (
  <RecordCenter {...props} />
);
