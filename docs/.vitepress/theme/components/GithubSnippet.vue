<script setup>
import { computed } from 'vue'

const props = defineProps({
  target: {
    type: String,
    required: true
  },
  style: {
    type: String,
    default: 'atom-one-dark'
  },
  height: {
    type: String,
    default: '434px'
  }
})

const iframeUrl = computed(() => {
  const params = new URLSearchParams({
    target: props.target,
    style: props.style,
    type: 'code',
    showBorder: 'on',
    showLineNumbers: 'on',
    showFileMeta: 'on',
    showFullPath: 'on',
    showCopy: 'on'
  })
  return `https://emgithub.com/iframe.html?${params.toString()}`
})
</script>

<template>
  <iframe
      :src="iframeUrl"
      frameborder="0"
      scrolling="no"
      :style="{ width: '100%', height }"
      allow="clipboard-write"
      class="github-snippet"
  ></iframe>
</template>

<style scoped>
.github-snippet {
  padding:  -100px -100px -100px 0;
  border-radius: 8px;
}

@media (max-width: 640px) {
  .github-snippet {
    margin: 12px -16px;
  }
}
</style>