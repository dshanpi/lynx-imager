import { formatErrorMessage } from '../../../Utils';
export function formatLearnError(error: unknown) {
  const structured =
    typeof error === 'object' && error !== null
      ? (error as { machineCode?: unknown; message?: unknown })
      : null;
  const message =
    typeof structured?.message === 'string' ? structured.message : formatErrorMessage(error);
  const [legacyCode] = message.split(':', 1);
  const code = typeof structured?.machineCode === 'string' ? structured.machineCode : legacyCode;
  return (
    (
      {
        AUTH_REQUIRED: '请先登录 100ASK 账号。',
        AUTH_TOKEN_EXPIRED: '登录已过期，请重新登录。',
        ENTITLEMENT_REQUIRED: '当前账号没有对应课程权益。',
        LEARNING_API_UNAVAILABLE: '课程服务暂不可用，请稍后重试。',
        LEARNING_API_INVALID_RESPONSE: '课程服务响应格式无效。',
        LEARNING_API_INVALID_JSON: '课程接口返回了网页或无效 JSON，请检查官网 API 转发。',
        LEARNING_API_SCHEMA_MISMATCH: '官网课程字段版本与当前 LYNX 不兼容，请升级或联系管理员。',
        LAB_PACKAGE_NOT_FOUND: '官网实验包尚未发布，请等待课程团队完成测试和发布。',
        LAB_PACKAGE_CREDENTIAL_INVALID: '实验包发布信息不完整，已阻止下载。',
        LEARNING_IDENTIFIER_INVALID: '课程或实验标识无效。',
        LEARNING_STATE_CONFLICT: '学习状态已更新，请同步后重试。',
      } as Record<string, string>
    )[code] || formatErrorMessage(error)
  );
}

/** These local checks have specific recovery paths; keep implementation codes out of the notice. */
export function formatLocalLearningError(error: unknown): string {
  const structured =
    typeof error === 'object' && error !== null ? (error as { message?: unknown }) : null;
  const message =
    typeof structured?.message === 'string' ? structured.message : formatErrorMessage(error);
  const code = message.match(
    /^(?:UnknownError:\s*)?(LEARNING_(?:INPUTS_(?:CHANGED|UNAVAILABLE|UNRECORDED)|ARTIFACT_(?:INVALID|NOT_FOUND|UNAVAILABLE)|ACTIVITY_(?:NOT_VISITED|ALREADY_CURRENT)|PREREQUISITES_REQUIRED|PREREQUISITE_INPUTS_CHANGED|ACTION_CONTEXT_CHANGED))(?::|$)/
  )?.[1];
  const notices: Record<string, string> = {
    LEARNING_PREREQUISITES_REQUIRED:
      '请先完成列出的前置活动。旧尝试通过、跳过或缺少有效证据不能解锁；教材和草稿仍保留。',
    LEARNING_PREREQUISITE_INPUTS_CHANGED:
      '前置活动的源码、绑定或环境已变化。请先重新验证前置活动，再继续本课；历史记录保留。',
    LEARNING_ACTIVITY_NOT_VISITED: '此课时尚未开始，可先查看教材；请按当前课程顺序进入实践。',
    LEARNING_ACTIVITY_ALREADY_CURRENT: '已在本课的当前尝试中，请继续下方学习步骤。',
    LEARNING_ACTION_CONTEXT_CHANGED:
      '学习位置或尝试已更新。请重新打开当前会话后选择课时；旧记录仍保留。',
    LEARNING_ARTIFACT_INVALID:
      '产物内容或记录校验失败，未创建预览或下载。请保留历史记录并核对课程输出；需要新产物时再显式重新实践。',
    LEARNING_ARTIFACT_NOT_FOUND:
      '此执行记录没有所选产物快照。请重新打开对应记录；不会从当前工程补造历史文件。',
    LEARNING_ARTIFACT_UNAVAILABLE:
      '无法保存课程声明的产物。请检查输出文件是否存在、可读取且未超出课程大小限制；修复后再重新实践。',
    LEARNING_INPUTS_CHANGED:
      '课程工程已变化，或本活动声明的 SDK、工具环境不符。旧结果不能用于继续验证；请在“工程与版本”检查差异，恢复原输入或点击“重新实践”创建新的尝试，历史记录会保留。',
    LEARNING_INPUTS_UNAVAILABLE:
      '暂时无法核验当前课程工程。请检查 Ubuntu 连接、文件可读性和工程大小；软链接或特殊文件目前无法核验。确认后可重试，或重新实践。',
    LEARNING_INPUTS_UNRECORDED:
      '这条旧记录没有完整的工程或所需环境快照。请点击“重新实践”生成当前证据；历史记录会保留。',
  };
  return (code && notices[code]) || formatErrorMessage(error);
}
