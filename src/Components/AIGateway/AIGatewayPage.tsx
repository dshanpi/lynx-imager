import React from 'react';
import { useTranslation } from 'react-i18next';
import { PageContainer } from '../../CoreUI';
import { AIGateway } from './AIGateway';

export const AIGatewayPage: React.FC = () => {
  const { t } = useTranslation();

  return (
    <PageContainer
      title={t('aiGateway.title', 'AI 通道')}
      description={t(
        'aiGateway.description',
        '为 Codex、Claude Code 和 MCP 客户端建立硬件能力通道'
      )}
    >
      <AIGateway />
    </PageContainer>
  );
};

export default AIGatewayPage;
