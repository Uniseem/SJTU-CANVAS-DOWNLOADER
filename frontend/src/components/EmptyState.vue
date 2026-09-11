<script setup lang="ts">
defineProps<{
  icon?: string
  title: string
  description: string
}>()
</script>

<template>
  <div class="empty-state text-center pa-8 pa-sm-12">
    <div class="empty-icon mx-auto mb-4">
      <v-icon :icon="icon ?? 'mdi-inbox-outline'" size="30" />
    </div>
    <h3>{{ title }}</h3>
    <p class="mt-2 mb-0 mx-auto">{{ description }}</p>
    <div v-if="$slots.default" class="mt-5"><slot /></div>
  </div>
</template>
