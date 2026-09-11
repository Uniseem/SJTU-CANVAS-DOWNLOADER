<script setup lang="ts">
defineProps<{
  eyebrow?: string
  title: string
  description?: string
}>()
</script>

<template>
  <header class="page-heading d-flex flex-wrap align-end justify-space-between ga-4">
    <div>
      <div v-if="eyebrow" class="eyebrow mb-2">{{ eyebrow }}</div>
      <h1>{{ title }}</h1>
      <p v-if="description" class="mt-2 mb-0">{{ description }}</p>
    </div>
    <div v-if="$slots.actions" class="heading-actions d-flex flex-wrap ga-2">
      <slot name="actions" />
    </div>
  </header>
</template>
