<template>
  <span class="game-icon" :aria-label="alt"><img v-if="src && !failed" :src="src" alt="" loading="lazy" decoding="async" @error="failed = true"/><span v-else aria-hidden="true">{{ fallback }}</span></span>
</template>
<script setup lang="ts">
import { ref, watch } from 'vue'
const props = withDefaults(defineProps<{ src?: string; alt?: string; fallback?: string }>(), { src: '', alt: '', fallback: '◇' })
const failed = ref(false)
watch(() => props.src, () => { failed.value = false })
</script>
<style scoped>
.game-icon{display:grid;place-items:center;width:100%;height:100%;overflow:hidden;color:#87b9c2;font-size:14px;line-height:1}.game-icon img{display:block;width:100%;height:100%;object-fit:cover}
</style>
