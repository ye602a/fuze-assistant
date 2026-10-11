<template>
  <svg viewBox="0 0 32 32" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="miter" aria-hidden="true" focusable="false">
    <template v-if="name === 'overview'"><path d="M16 3 28 10v12L16 29 4 22V10Z"/><path d="m16 9 7 7-7 7-7-7Z M16 3v6 M28 10l-5 6 M4 22l5-6"/></template>
    <template v-else-if="name === 'history'"><path d="M8 5h17v23H8L4 24V9Z M8 5v23 M12 11h9 M12 16h9 M12 21h5"/><path d="m22 3 4 4 4-4"/></template>
    <template v-else-if="name === 'settings'"><path d="M16 3v7m0 12v7M3 16h7m12 0h7M7 7l5 5m8 8 5 5M7 25l5-5m8-8 5-5"/><path d="m16 9 7 7-7 7-7-7Z"/></template>
    <template v-else-if="name === 'network'"><path d="m8 5 8 8-5 5-8-8Z m13 9 8 8-5 5-8-8Z M11 21l10-10 M21 3l8 8 M3 21l8 8"/></template>
    <template v-else-if="name === 'capture'"><path d="M12 5H5v7m15-7h7v7M5 20v7h7m15-7v7h-7M3 16h26"/><path d="m16 10 6 6-6 6-6-6Z"/></template>
    <template v-else-if="name === 'recognition'"><path d="M8 5h16M16 3v8M7 11h18l-4 8-5 4-5-4Z M9 29l7-6 7 6"/><path d="m12 13 4 6 4-6"/></template>
    <template v-else-if="name === 'data'"><path d="m16 3 12 6-12 6L4 9Z M4 15l12 6 12-6M4 21l12 6 12-6"/></template>
    <template v-else><path d="m16 2 10 9v10L16 30 6 21V11Z M16 7v18 M10 12l6 4 6-4 M10 20l6-4 6 4"/></template>
  </svg>
</template>
<script setup lang="ts">
defineProps<{ name?: string }>()
</script>
