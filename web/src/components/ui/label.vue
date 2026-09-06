<script setup lang="ts">
import type { HTMLAttributes } from "vue"
import { cn } from "@/lib/utils"

interface Props {
  for?: string
  htmlFor?: string
  class?: HTMLAttributes["class"]
  className?: HTMLAttributes["class"]
}

const props = defineProps<Props>()
</script>

<template>
  <label
    data-slot="label"
    :for="props.for || props.htmlFor"
    :class="
      cn(
        'flex items-center gap-2 text-sm leading-none font-medium select-none group-data-[disabled=true]:pointer-events-none group-data-[disabled=true]:opacity-50 peer-disabled:cursor-not-allowed peer-disabled:opacity-50',
        props.class,
        props.className
      )
    "
  >
    <slot />
  </label>
</template>
