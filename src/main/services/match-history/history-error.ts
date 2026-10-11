import type { MatchHistoryIssue } from '../../../shared/ipc-contract.ts'

export class HistoryError extends Error {
  constructor(readonly issue: MatchHistoryIssue, readonly status?: number) {
    super(issue)
    this.name = 'HistoryError'
  }
}

export function historyIssue(error: unknown): MatchHistoryIssue {
  return error instanceof HistoryError ? error.issue : 'service-unavailable'
}

export function httpHistoryError(status?: number): HistoryError {
  const issue = status === 401 ? 'unauthorized'
    : status === 403 ? 'forbidden'
      : status === 429 ? 'rate-limited'
        : status === 404 ? 'invalid-response' : 'service-unavailable'
  return new HistoryError(issue, status)
}
