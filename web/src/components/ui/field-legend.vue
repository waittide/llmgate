<script setup lang="ts">
import type { HTMLAttributes } from "vue"
import { cn } from "@/lib/utils"

interface Props {
  variant?: "legend" | "label"
  class?: HTMLAttributes["class"]
  className?: HTMLAttributes["class"]
}

const props = withDefaults(defineProps<Props>(), {
  variant: "legend",
})
</script>

<template>
  <legend
    data-slot="field-legend"
    :data-variant="variant"
    :class="
      cn(
        'mb-3 font-medium data-[variant=legend]:text-base data-[variant=label]:text-sm',
        props.class,
        props.className
      )
    "
  >
    <slot />
  </legend>
</template>
