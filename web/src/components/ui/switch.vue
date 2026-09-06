<script setup lang="ts">
import { computed, type HTMLAttributes } from "vue"
import { cn } from "@/lib/utils"

const modelValue = defineModel<boolean>()

interface Props {
  checked?: boolean
  defaultChecked?: boolean
  disabled?: boolean
  required?: boolean
  id?: string
  name?: string
  value?: string
  class?: HTMLAttributes["class"]
  className?: HTMLAttributes["class"]
}

const props = withDefaults(defineProps<Props>(), {
  disabled: false,
})

const emit = defineEmits<{
  "update:checked": [checked: boolean]
  checkedChange: [checked: boolean]
}>()

const isChecked = computed(() => {
  if (modelValue.value !== undefined) {
    return Boolean(modelValue.value)
  }
  if (props.checked !== undefined) {
    return Boolean(props.checked)
  }
  return Boolean(props.defaultChecked)
})

function toggle() {
  if (props.disabled) return
  const next = !isChecked.value
  modelValue.value = next
  emit("update:checked", next)
  emit("checkedChange", next)
}
</script>

<template>
  <button
    type="button"
    role="switch"
    :id="id"
    :name="name"
    :value="value"
    :aria-checked="isChecked ? 'true' : 'false'"
    :aria-required="required ? 'true' : undefined"
    :data-state="isChecked ? 'checked' : 'unchecked'"
    :disabled="disabled"
    data-slot="switch"
    :class="
      cn(
        'peer data-[state=checked]:bg-primary data-[state=unchecked]:bg-input focus-visible:border-ring focus-visible:ring-ring/50 dark:data-[state=unchecked]:bg-input/80 inline-flex h-[1.15rem] w-8 shrink-0 items-center rounded-full border border-transparent shadow-xs transition-all outline-none focus-visible:ring-[3px] disabled:cursor-not-allowed disabled:opacity-50',
        props.class,
        props.className
      )
    "
    @click="toggle"
  >
    <span
      data-slot="switch-thumb"
      :data-state="isChecked ? 'checked' : 'unchecked'"
      :class="
        cn(
          'bg-background dark:data-[state=unchecked]:bg-foreground dark:data-[state=checked]:bg-primary-foreground pointer-events-none block size-4 rounded-full ring-0 transition-transform data-[state=checked]:translate-x-[calc(100%-2px)] data-[state=unchecked]:translate-x-0'
        )
      "
    />
  </button>
</template>
