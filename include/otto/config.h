#ifndef OTTO_CONFIG_H
#define OTTO_CONFIG_H

#include <stddef.h>

#include "otto/common.h"

typedef struct {
    char *name;
    char *baseurl;
    char *apikey;
    char *model;
} OttoConfig;

void otto_config_init(OttoConfig *config);
void otto_config_free(OttoConfig *config);

OttoExitCode otto_config_get_path(char **path);
OttoExitCode otto_config_get_directory(char **directory);
OttoExitCode otto_config_load(
    const char *path,
    OttoConfig *config,
    int *found
);
OttoExitCode otto_config_validate(
    const OttoConfig *config,
    char *message,
    size_t message_size
);
OttoExitCode otto_config_save_atomic(
    const char *path,
    const OttoConfig *config
);

OttoExitCode otto_config_save_values(
    const char *path,
    const char *name,
    const char *baseurl,
    const char *apikey,
    const char *model
);

OttoExitCode otto_config_interactive(const char *path);

#endif
