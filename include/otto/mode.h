#ifndef OTTO_MODE_H
#define OTTO_MODE_H

#include "otto/common.h"

OttoExitCode otto_mode_get_active(char **mode, int *found);
OttoExitCode otto_mode_set_active(const char *mode);

OttoExitCode otto_mode_combine_prompts(
    const char *base_prompt,
    const char *mode_prompt,
    char **combined_prompt
);

OttoExitCode otto_mode_load(
    const char *mode,
    char **system_prompt,
    int *found
);

#endif
