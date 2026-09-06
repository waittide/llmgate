<script setup lang="ts">
import type { HTMLAttributes } from "vue"
import { cn } from "@/lib/utils"
import { fieldVariants, type FieldOrientation } from "./field"

interface Props {
  orientation?: FieldOrientation
  class?: HTMLAttributes["class"]
  className?: HTMLAttributes["class"]
}

const props = withDefaults(defineProps<Props>(), {
  orientation: "vertical",
})
</script>

<template>
  <div
    role="group"
    data-slot="field"
    :data-orientation="orientation"
    :class="cn(fieldVariants({ orientation }), props.class, props.className)"
  >
    <slot />
  </div>
</template>
