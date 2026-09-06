<script setup lang="ts">
import type { Component, HTMLAttributes } from "vue"
import { cn } from "@/lib/utils"
import { buttonVariants, type ButtonVariant, type ButtonSize } from "./button"

interface Props {
  variant?: ButtonVariant
  size?: ButtonSize
  as?: string | Component
  type?: "button" | "submit" | "reset"
  disabled?: boolean
  class?: HTMLAttributes["class"]
  className?: HTMLAttributes["class"]
}

const props = withDefaults(defineProps<Props>(), {
  variant: "default",
  size: "default",
  as: "button",
  type: "button",
})
</script>

<template>
  <component
    :is="as"
    :type="as === 'button' ? type : undefined"
    data-slot="button"
    :data-variant="variant"
    :data-size="size"
    :disabled="disabled"
    :class="cn(buttonVariants({ variant, size }), props.class, props.className)"
  >
    <slot />
  </component>
</template>
