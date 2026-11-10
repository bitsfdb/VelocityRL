cmake_minimum_required(VERSION 3.16)
project(rlim_camber_underglow_plugin C)

set(CMAKE_C_STANDARD 11)

include_directories(../../include)

add_library(rlim_camber_underglow_plugin SHARED
    plugin_main.c
)

set_target_properties(rlim_camber_underglow_plugin PROPERTIES
    PREFIX ""
    OUTPUT_NAME "rlim_camber_underglow"
)
