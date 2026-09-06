<script setup lang="ts">
import { computed, type HTMLAttributes, useSlots } from "vue"
import { cn } from "@/lib/utils"

interface Props {
  errors?: Array<{ message?: string } | undefined>
  class?: HTMLAttributes["class"]
  className?: HTMLAttributes["class"]
}

const props = defineProps<Props>()
const slots = useSlots()

const uniqueErrors = computed(() => {
  if (!props.errors?.length) return []
  const map = new Map<string, { message?: string }>()
  for (const err of props.errors) {
    if (err?.message) {
      map.set(err.message, err)
    }
  }
  return Array.from(map.values())
})

const hasSlot = computed(() => !!slots.default)
const hasErrors = computed(() => uniqueErrors.value.length > 0)
</script>

<template>
  <div
    v-if="hasSlot || hasErrors"
    role="alert"
    data-slot="field-error"
    :class="cn('text-destructive text-sm font-normal', props.class, props.className)"
  >
    <slot v-if="hasSlot" />
    <template v-else-if="uniqueErrors.length === 1">
      {{ uniqueErrors[0]?.message }}
    </template>
    <ul v-else-if="uniqueErrors.length > 1" class="ml-4 flex list-disc flex-col gap-1">
      <li v-for="(error, index) in uniqueErrors" :key="index">
        {{ error.message }}
      </li>
    </ul>
  </div>
</template>
