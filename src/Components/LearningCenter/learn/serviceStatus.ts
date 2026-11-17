import type { LearnCapabilitiesResult } from '../../../Platform/IPC/Commands';

export function learnServiceMessage(result: LearnCapabilitiesResult): string {
  if (result.availability === 'not_deployed') {
    return '官网暂未提供新版课程接口，现有课程继续按原接口使用。';
  }
  if (result.missingCapabilities.length > 0) {
    return '官网新版接口可连接，但当前 LYNX 尚未支持服务要求的全部能力，请等待对应客户端更新。';
  }
  return '官网新版接口可连接。此检查不代表课程、实验执行和学习记录同步已经全部接通。';
}

export function learnServiceError(error: unknown): string {
  const code =
    typeof error === 'object' && error !== null && 'machineCode' in error
      ? String(error.machineCode)
      : '';
  const messages: Record<string, string> = {
    LEARN_EVIDENCE_STALE:
      '当前工程或环境已变化。请重新执行相关动作，预览并同步新证据后再评测；历史记录仍保留。',
    LEARN_EVIDENCE_CHECK_UNAVAILABLE:
      '暂时无法核验当前工程。请检查 SSH 连接和工程绑定后重试；历史记录仍保留。',
    LEARN_SYNC_PENDING: '本课时还有证据待确认同步，请先完成原批次，再提交评测。',
    LEARN_SYNC_ATTEMPT_MISMATCH: '实践尝试已更新，请先点击“同步评测结果”，再核对并提交。',
    LEARN_REVIEW_PENDING: '有一次评测尚未确认，请先在评测区恢复原提交，再新建实践同步批次。',

    LEARN_OPERATION_IN_PROGRESS:
      '原提交仍在服务器处理中。已保留原输入和请求标识，请稍后重试原提交确认结果。',
    LEARN_WRITE_UNCONFIRMED:
      '暂时无法确认原提交结果。已保留恢复记录，重试会使用同一请求，不会用新输入替换。',
    LEARN_IDEMPOTENCY_CONFLICT:
      '原请求标识已被用于不同内容。请保留本机恢复记录并核对，不能自动创建新标识覆盖。',
    LEARN_EXECUTION_CONTEXT_INVALID:
      '执行环境或源码输入记录校验失败，不能把当前环境倒填为历史记录。请保留原任务并核对。',
    LEARN_EVIDENCE_NOT_AVAILABLE:
      '此任务还没有可收集的完整执行记录。请查询实际状态；旧任务缺少时间或独立输出时不能补造证据。',
    LEARN_RUN_EVIDENCE_NOT_AVAILABLE:
      '远端未保存完整执行证据，请保留原任务记录并核对。不会自动重新执行。',
    LEARN_EVIDENCE_INVALID:
      '证据内容、范围或摘要校验失败，未保存新的快照。请查询原任务并核对完整输出。',
    LEARN_EVIDENCE_CHANGED:
      '收集期间证据或本地快照发生变化，已阻止混合保存。请重新打开原任务记录。',
    LEARN_RUN_EVIDENCE_CHANGED: '读取期间远端证据发生变化，未保存部分内容。请查询原任务后重试。',
    LEARN_WRITE_RECOVERY_UNAVAILABLE:
      '无法保存或清除本机提交恢复记录。请检查存储后同步；尚未确认的提交不会自动重发。',
    LEARN_WRITE_RECOVERY_INVALID:
      '本机待确认提交格式无效、损坏或输入超限。请保留本机数据并核对输入，未发送新的提交。',
    LEARN_WRITE_PENDING: '存在另一份待确认提交。请同步并处理原请求，未覆盖原输入。',
    LEARN_REPORT_RECOVERY_UNAVAILABLE:
      '无法保存或读取报告恢复记录。请检查本机存储后重试，原请求不会自动重新创建。',
    LEARN_REPORT_RECOVERY_INVALID:
      '本机报告恢复记录损坏，请保留本机数据并联系支持。未自动创建新报告。',
    LEARN_REPORT_MISMATCH: '报告与本次请求的课程、范围或记录版本不一致，已阻止显示和导出。',
    LEARN_REPORT_FAILED: '报告生成失败。可以同步评测结果后重新生成，原任务记录仍保留。',
    LEARN_REPORT_CANCELLED: '报告任务已取消。可以同步评测结果后重新生成。',
    LEARN_RUN_CONFLICT: '任务记录已更新。请查询原任务的实际状态，不要重复执行。',
    LEARN_SYNC_CONFLICT: '云端尝试或证据归属存在冲突，原请求已保留。请核对后继续原批次。',
    LEARN_SYNC_RUN_ALREADY_CLAIMED: '此执行已属于一个同步批次。请刷新同步记录并继续原批次。',
    LEARN_SYNC_SELECTION_INCOMPATIBLE:
      '所选执行的课程、动作、工程或执行环境不一致。请按同一环境重新选择，每个动作只选一次执行。',
    LEARN_SYNC_CONTEXT_MISSING:
      '此旧执行没有保存当时的环境观察，只能保留为本地历史，不能补造后上传。',
    LEARN_SYNC_PREVIEW_REQUIRED: '请先查看并保存此次执行的完整证据，再选择上传。',
    LEARN_SYNC_RECEIPT_INVALID: '云端回执与原批次不匹配，未推进同步。原请求和证据已保留。',
    LEARN_SYNC_JOURNAL_INVALID:
      '本机同步记录无法通过完整性检查。请保留本机数据并联系支持，未重建请求。',
    LEARN_SYNC_NOT_FOUND: '当前账号下没有这个同步批次，请刷新本课时的同步记录。',
    LEARN_SYNC_NOT_PERMITTED: '当前账号或课程权限不允许同步此批证据，请重新核对课程权益。',
    LEARN_SYNC_REJECTED: '官网明确拒绝了此次证据请求。原批次已保留，请核对课程和证据要求。',
    LEARN_UPLOAD_GRANT_INVALID: '上传授权与原证据不匹配，文件尚未发送，请核对官网存储配置。',
    LEARN_UPLOAD_ORIGIN_INVALID: '上传地址未通过安全校验，文件尚未发送，请核对官网存储配置。',
    LEARN_UPLOAD_DNS_UNAVAILABLE: '暂时无法解析文件存储地址。原批次已保留，可以稍后继续。',
    LEARN_UPLOAD_UNCONFIRMED:
      '尚不能确认文件上传结果。继续原批次会重试相同内容，不会重新执行课程任务。',
    LEARN_UPLOAD_EXPIRED: '文件上传授权已过期。继续原批次将恢复授权并上传相同证据。',
    LEARN_STORE_UNAVAILABLE:
      '无法读取或保存本机学习记录，请重试；持续失败时检查磁盘空间和文件权限。',
    LEARN_RUN_SOURCE_CHANGED: '工程输入或执行环境在准备后发生变化，请重新准备任务并核对参数。',
    LEARN_RUN_COMMIT_MISMATCH: '工程 HEAD 与课程固定提交不一致，请先核对源码版本。',
    LEARN_RUN_ENVIRONMENT_VERSION_MISMATCH: 'Ubuntu 实际版本与课程要求不一致，请先核对环境要求。',
    LEARN_RUN_ENVIRONMENT_CHECK_UNSUPPORTED:
      '此课程要求的 SDK、镜像或工具链核验器尚未接通，不能跳过检查执行。',
    LEARN_RUN_TASK_DEFINITION_CHANGED:
      '工程任务定义与课程固定版本不一致，不能执行。请恢复已审核的任务文件。',
    LEARN_RUN_TASK_NOT_FOUND: '当前固定源码中没有此任务，请联系课程管理员核对 .lynx/tasks.json。',
    LEARN_RUN_GIT_FAILED: '无法读取固定版本的工程任务，请检查 Git、源码提交和 .lynx/tasks.json。',
    LEARN_RUN_SOURCE_BYTES_LIMIT: '工程源码超出核验上限，请绑定课程指定的子工程仓库。',
    LEARN_RUN_CONNECTION_LOST: 'SSH 连接中断，任务可能仍在执行。请查询原任务，不要重复启动。',
    LEARN_RUN_RESPONSE_INVALID:
      '没有收到可核验的任务状态。请检查 Ubuntu 的 Python 3，再查询原任务。',
    LEARN_RUN_ALREADY_DISPATCHED: '此任务已经发出执行请求。请查询原任务，不能重复启动。',
    LEARN_RUN_NOT_FOUND: '未找到此账号的任务记录，请重新打开课程并同步。',
    LEARN_ACTION_UNSUPPORTED: '此活动需要尚未开放的执行适配器，请更新客户端或联系课程管理员。',
    LEARN_RUN_PATH_ESCAPE: '任务引用了工程外路径，已阻止执行。请联系课程管理员修正任务。',
    LEARN_BINDING_REQUIRED: '请先选择并保存课程工程。',
    LEARN_BINDING_CHANGED: '工程、主机或板卡配置已变化，请重新确认绑定。',
    LEARN_PROJECT_MISSING: '绑定工程已移除，请选择现有工程。',
    LEARN_BOARD_REQUIRED: '请先在 Linux 工作台为此工程保存板卡绑定。',
    LEARN_INSPECT_FAILED: '检查失败：请核对 Ubuntu 工程目录、Git、Python 3 和读取权限。',
    LEARN_IDE_ALIAS_REQUIRED: '非默认端口或特殊地址需要填写 SSH 别名。',
    LEARN_IDE_ALIAS_MISMATCH: 'SSH 别名与已绑定主机、用户或端口不匹配；请核对明确的 Host 配置。',
    LEARN_IDE_CONFIG_MISSING: '未找到本机 SSH 配置，请先在 VS Code Remote SSH 中配置主机。',
    LEARN_IDE_OPEN_FAILED: '无法打开 VS Code，请确认已安装 VS Code 和 Remote SSH 扩展。',
    LEARN_IDE_PATH_INVALID: '文件须是工程内的相对路径，不能包含上级目录。',
    LEARN_ACCOUNT_CHANGED: '账号已切换，请重新打开课程。',
    HOST_KEY_CONFIRM_REQUIRED: '请先在 Linux 工作台核验并保存 Ubuntu 的 SSH 主机指纹。',
    HOST_KEY_CHANGED: 'Ubuntu 主机指纹发生变化，请先在 Linux 工作台核实。',
    SSH_AUTH: 'SSH 认证失败，请在 Linux 工作台检查已保存的凭据。',
    SSH_CONNECT: '无法连接 Ubuntu，请检查网络和主机状态。',
    LEARN_REVIEW_CONFLICT: '学习记录或尝试已更新，请同步后重新确认提交。',
    LEARN_RECORD_CONFLICT: '学习记录已由另一端更新，请同步后再保存。',
    LEARN_RECORD_LIMIT: '课程笔记总量超出限制，请导出整理后再保存。',
    LEARN_RECORD_NOT_ALLOWED: '此活动需要实操证据，或复盘内容为空。',
    LEARN_PREVIEW_MISMATCH: '试学账号、版本或回执不匹配，请重新打开邀请。',
    LEARN_PREVIEW_STALE: '草稿或请求已变化，请在官网重新生成试学邀请。',
    LEARN_PREVIEW_EXPIRED: '试学邀请已过期，请在官网重新生成。',
    LEARN_PREVIEW_LIMIT: '教材联调规模超限，请拆分课程预览。',
    LEARN_CHECK_TIMEOUT: '教材联调超时。可重试核对同一请求的回执，不会重复执行实验。',
    LEARN_SIGNER_NOT_CONFIGURED: '当前客户端尚未配置新版课程官方公钥，请更新官方客户端。',
    LEARN_SIGNER_CONFIG_INVALID: '课程官方公钥配置无效，请联系发布管理员。',
    LEARN_SIGNER_UNTRUSTED: '课程签名来源不受信任，已阻止打开。',
    LEARN_SIGNATURE_INVALID: '课程签名验证失败，已阻止打开。',
    LEARN_DIGEST_MISMATCH: '课程内容摘要不匹配，已阻止打开。',
    LEARN_DOCUMENT_MISMATCH: '教材快照与签名版本不一致，请重新同步。',
    LEARN_UNIT_MISMATCH: '教材不属于当前课程章节，已阻止显示。',
    LEARN_RELEASE_MISMATCH: '课程发布版本发生变化，请返回课程列表重新同步。',
    LEARN_ENTITLEMENT_REQUIRED: '当前账号没有这门课程的有效权益，请重新同步。',
    LEARN_MANIFEST_UNSUPPORTED: '课程清单版本不受支持，请更新客户端。',
    LEARN_NOT_FOUND: '课程或课时已不可访问，请重新同步课程。',
    LEARN_AUTH_REQUIRED: '请先登录官网账号，再检查课程连接。',
    LEARN_FORBIDDEN: '当前账号无权访问新版课程接口。',
    LEARN_RATE_LIMITED: '官网请求频率受限，请稍后重试。',
    LEARN_SERVICE_UNAVAILABLE: '官网课程服务暂不可用，请稍后重试。',
    LEARN_NETWORK_UNAVAILABLE: '无法连接官网课程服务，请检查网络。',
    LEARN_ORIGIN_UNTRUSTED: '课程连接地址不在受信官网范围内，请管理员核对配置。',
    LEARN_REDIRECT_REJECTED: '课程接口发生重定向，请管理员核对部署。',
    LEARN_SCHEMA_MISMATCH: '课程接口格式不兼容，请管理员核对协议。',
    LEARN_PROTOCOL_UNSUPPORTED: '官网未声明支持当前学习接口协议。',
    LEARN_RESPONSE_TOO_LARGE: '课程接口响应超出限制，请管理员检查。',
    LEARN_RESPONSE_INTERRUPTED: '官网响应中断，请稍后重试。',
  };
  return messages[code] || '课程连接检查失败，请稍后重试。';
}
