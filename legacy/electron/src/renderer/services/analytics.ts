// Personal build: do not load Firebase or send telemetry.
export async function initRendererAnalytics(..._args: unknown[]) { return null }
export function trackPageView(..._args: unknown[]) {}
export function trackAnalyticsEvent(..._args: unknown[]) {}
export function trackErrorEvent(..._args: unknown[]) {}
