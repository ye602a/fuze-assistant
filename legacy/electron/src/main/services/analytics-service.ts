// Personal fork: no tracking identifiers, queue, network or persisted telemetry.
export async function getAnalyticsStatus() { return { enabled: false, configured: false, pendingEvents: [] } }
export const initAnalyticsService = getAnalyticsStatus
export async function setAnalyticsEnabled(_enabled?: unknown) { return getAnalyticsStatus() }
export async function trackAnalyticsEvent() { return { success: true, skipped: true } }
export function recordPendingAnalyticsEvent(..._args: unknown[]) {}
export function markAnalyticsAppStarted(..._args: unknown[]) {}
export function markAnalyticsAppCleanExit() {}
