/**
 * Formats error object to message string.
 *
 * Extracts message from various error types including
 * Error instances and objects with 'message' or 'error'
 * properties. Returns stringified value for unknown types.
 *
 * @param error - Error object or value to format
 * @returns Human-readable error message string
 */
export function formatErrorMessage(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }
  if (error && typeof error === 'object') {
    if ('message' in error && typeof (error as { message: unknown }).message === 'string') {
      const { message } = error as { message: string };
      const kind =
        ('name' in error && typeof (error as { name: unknown }).name === 'string'
          ? (error as { name: string }).name
          : undefined) ??
        ('code' in error && typeof (error as { code: unknown }).code === 'string'
          ? (error as { code: string }).code
          : undefined);
      return kind ? `${kind}: ${message}` : message;
    }
    if ('error' in error && typeof (error as { error: unknown }).error === 'string') {
      return (error as { error: string }).error;
    }
    if ('error' in error && (error as { error: unknown }).error !== error) {
      return formatErrorMessage((error as { error: unknown }).error);
    }
    try {
      const serialized = JSON.stringify(error);
      if (serialized) return serialized;
    } catch {
      // Fall through to a stable type label for circular objects.
    }
    return Object.prototype.toString.call(error);
  }
  return String(error);
}
