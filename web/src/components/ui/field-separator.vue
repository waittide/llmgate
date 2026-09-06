<script setup lang="ts">
import type { HTMLAttributes } from "vue"
import { cn } from "@/lib/utils"

interface Props {
  class?: HTMLAttributes["class"]
  className?: HTMLAttributes["class"]
}

const props = defineProps<Props>()
</script>

<template>
  <div
    data-slot="field-separator"
    :data-content="!!$slots.default"
    :class="
      cn(
        'relative -my-2 h-5 text-sm group-data-[variant=outline]/field-group:-mb-2',
        props.class,
        props.className
      )
    "
  >
    <div
      data-slot="separator"
      role="none"
      class="bg-border shrink-0 h-px w-full absolute inset-0 top-1/2"
    />
    <span
      v-if="$slots.default"
      class="bg-background text-muted-foreground relative mx-auto block w-fit px-2"
      data-slot="field-separator-content"
    >
      <slot />
    </span>
  </div>
</template>
