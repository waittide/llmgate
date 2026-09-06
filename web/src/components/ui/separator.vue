<script setup lang="ts">
import type { HTMLAttributes } from "vue"
import { cn } from "@/lib/utils"

interface Props {
  orientation?: "horizontal" | "vertical"
  decorative?: boolean
  class?: HTMLAttributes["class"]
  className?: HTMLAttributes["class"]
}

const props = withDefaults(defineProps<Props>(), {
  orientation: "horizontal",
  decorative: true,
})
</script>

<template>
  <div
    data-slot="separator"
    :data-orientation="orientation"
    :role="decorative ? 'none' : 'separator'"
    :class="
      cn(
        'bg-border shrink-0 data-[orientation=horizontal]:h-px data-[orientation=horizontal]:w-full data-[orientation=vertical]:h-full data-[orientation=vertical]:w-px',
        props.class,
        props.className
      )
    "
  />
</template>
