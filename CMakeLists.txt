cmake_minimum_required(VERSION 3.27.0...4.3.0)
project(libchewing LANGUAGES C)

set(CMAKE_PROJECT_VERSION 0.14.0-alpha.3)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

set(CMAKE_C_STANDARD 99)
set(CMAKE_C_EXTENSIONS ON) # require GNU extensions

find_package(Git)
if(Git_FOUND)
    execute_process(
        COMMAND ${GIT_EXECUTABLE} --work-tree ${CMAKE_SOURCE_DIR} describe --dirty --always
        OUTPUT_STRIP_TRAILING_WHITESPACE
        OUTPUT_VARIABLE GIT_DESCRIBE
        ERROR_VARIABLE GIT_DESCRIBE_ERROR
    )
    if(NOT GIT_DESCRIBE_ERROR)
        string(REPLACE "v" "" CMAKE_PROJECT_VERSION ${GIT_DESCRIBE})
    endif()
endif()

set(LIBCHEWING_VERSION ${CMAKE_PROJECT_VERSION})
set(PACKAGE_VERSION ${CMAKE_PROJECT_VERSION})
set(LIBCHEWING_BINARY_VERSION 1.0.0)
set(CMAKE_MODULE_PATH ${PROJECT_SOURCE_DIR}/cmake)

include(CTest)
set(CTEST_PARALLEL_LEVEL 1)

if(UNIX)
    add_compile_definitions(UNDER_POSIX PIC)
endif()

include(CheckCCompilerFlag)
include(FetchContent)
include(CMakeDependentOption)

option(BUILD_SHARED_LIBS "Build using shared libraries" ON)
option(USE_SYSTEM_CHEWING_CLI "Use system-installed chewing-cli to build dictionary" OFF)
option(BUILD_DOC "Build documentation" ON)
option(BUILD_DATA "Build chewing dictionaries" ON)
cmake_dependent_option(
    USE_BUNDLED_STATIC_LM
    "Use bundled static_lm.arpa file"
    ON
    "EXISTS ${CMAKE_SOURCE_DIR}/data/dict/chewing_v4/static_lm.arpa"
    OFF
)

if(CMAKE_C_COMPILER_ID MATCHES GNU|Clang)
    option(ENABLE_GCOV "Coverage support" OFF)
endif()

add_compile_options("$<$<C_COMPILER_ID:GNU,Clang,AppleClang>:-Wall;-finput-charset=utf-8>")
add_compile_options("$<$<C_COMPILER_ID:MSVC>:/utf-8>")

if(ENABLE_GCOV)
    add_compile_options("$<$<C_COMPILER_ID:GNU>:--coverage>")
    add_compile_options("$<$<C_COMPILER_ID:AppleClang,Clang>:-fprofile-instr-generate;-fcoverage-mapping>")
endif()

add_compile_definitions(HAVE_CONFIG_H=1)
add_custom_target(check COMMAND ${CMAKE_CTEST_COMMAND})

# Use valgrind when testing
option(USE_VALGRIND "Use valgrind when testing" true)

find_package(Corrosion QUIET)
if(NOT Corrosion_FOUND)
    FetchContent_Declare(
        Corrosion
        GIT_REPOSITORY https://github.com/corrosion-rs/corrosion.git
        GIT_TAG 1499b14e4906a2890f5cee1547c8848db261753d # v0.6.1
        FIND_PACKAGE_ARGS
    )
    FetchContent_MakeAvailable(Corrosion)
endif()

corrosion_import_crate(MANIFEST_PATH Cargo.toml CRATES chewing_capi)
if(BUILD_TESTING)
    corrosion_import_crate(MANIFEST_PATH Cargo.toml CRATES chewing_testhelper)
endif()
corrosion_import_crate(MANIFEST_PATH Cargo.toml CRATES chewing-cli)

if(ENABLE_GCOV)
    corrosion_set_env_vars(chewing_capi CARGO_INCREMENTAL=0)
    corrosion_add_target_local_rustflags(chewing_capi -Cinstrument-coverage -Ccodegen-units=1 -Cinline-threshold=0 -Clink-dead-code -Coverflow-checks=off -Cpanic=abort)
endif()

if(CMAKE_SYSTEM MATCHES "Windows")
    set(CMAKE_MSVC_RUNTIME_LIBRARY "MultiThreaded")
    corrosion_add_target_rustflags(chewing_capi -Ctarget-feature=+crt-static)
    if(BUILD_TESTING)
        corrosion_add_target_rustflags(chewing_testhelper -Ctarget-feature=+crt-static)
    endif()
    corrosion_add_target_rustflags(chewing-cli -Ctarget-feature=+crt-static)
    if(VCPKG_TARGET_TRIPLET)
        corrosion_set_env_vars(chewing-cli VCPKGRS_TRIPLET=${VCPKG_TARGET_TRIPLET})
    endif()
endif()

# Feature probe
include(CheckTypeSize)
check_type_size(uint16_t UINT16_T)

set(CURSES_NEED_WIDE true)
find_package(Curses)

set(SRC_DIR ${PROJECT_SOURCE_DIR}/src)
set(INC_DIR ${PROJECT_SOURCE_DIR}/capi/include)
set(TOOLS_SRC_DIR ${PROJECT_SOURCE_DIR}/src/tools)
set(TOOLS_BIN_DIR ${PROJECT_BINARY_DIR}/src/tools)
set(TEST_SRC_DIR ${PROJECT_SOURCE_DIR}/tests)
set(TEST_BIN_DIR ${PROJECT_BINARY_DIR}/tests)

include(GNUInstallDirs)

configure_file(
    ${PROJECT_SOURCE_DIR}/cmake/config.h.in
    ${PROJECT_BINARY_DIR}/include/config.h
)

configure_file(
    ${PROJECT_SOURCE_DIR}/cmake/version.texi.in
    ${PROJECT_BINARY_DIR}/doc/version.texi
)

set(prefix "${CMAKE_INSTALL_PREFIX}")
set(exec_prefix "\${prefix}")
set(libdir "\${exec_prefix}/${CMAKE_INSTALL_LIBDIR}")
set(includedir "\${prefix}/${CMAKE_INSTALL_INCLUDEDIR}")
set(datarootdir "\${prefix}/${CMAKE_INSTALL_DATADIR}")
set(datadir "\${datarootdir}")
set(sysconfdir "\${prefix}/${CMAKE_INSTALL_SYSCONFDIR}")
configure_file(
    ${PROJECT_SOURCE_DIR}/chewing.pc.in
    ${PROJECT_BINARY_DIR}/chewing.pc
    @ONLY
)

include_directories(
    ${PROJECT_BINARY_DIR}/include
    ${PROJECT_SOURCE_DIR}/capi/include
)

set(ALL_INC
    ${INC_DIR}/chewing.h

    # 0.5.x compatibility headers
    ${INC_DIR}/chewing-compat.h
    ${INC_DIR}/chewingio.h
    ${INC_DIR}/global.h
    ${INC_DIR}/mod_aux.h
)

if(NOT USE_SYSTEM_CHEWING_CLI)
    set(CHEWING_CLI $<TARGET_FILE:chewing-cli>)
endif()

if(BUILD_DOC)
    add_subdirectory(doc)
endif()

if(BUILD_DATA)
    add_subdirectory(data)
endif()

if(BUILD_TESTING)
    add_subdirectory(tests)
endif()

add_library(libchewing ${ALL_INC} capi/src/chewing.c)
set_target_properties(libchewing PROPERTIES LINKER_LANGUAGE C)
target_include_directories(libchewing
    PUBLIC
        $<BUILD_INTERFACE:${INC_DIR}>
        $<INSTALL_INTERFACE:${CMAKE_INSTALL_INCLUDEDIR}/chewing>
)

corrosion_set_env_vars(chewing_capi
    CHEWING_DATADIR=${CMAKE_INSTALL_FULL_DATADIR}/libchewing
)
target_link_libraries(libchewing PRIVATE chewing_capi)
if(BUILD_SHARED_LIBS)
    if(CMAKE_HOST_WIN32 AND CMAKE_C_COMPILER_ID MATCHES ^Clang)
        # llvm-mingw target
        target_link_options(libchewing
            PRIVATE ${PROJECT_SOURCE_DIR}/capi/src/symbols-msvc.def
        )
        set_target_properties(libchewing PROPERTIES
            LINK_DEPENDS ${PROJECT_SOURCE_DIR}/capi/src/symbols-msvc.def
        )
    elseif(CMAKE_C_COMPILER_ID MATCHES GNU|^Clang)
        target_link_options(libchewing
            PRIVATE LINKER:-version-script,${PROJECT_SOURCE_DIR}/capi/src/symbols-elf.map
            PRIVATE LINKER:--gc-sections
            PRIVATE LINKER:-u,chewing_new
            PRIVATE LINKER:-u,chewing_version
        )
        set_target_properties(libchewing PROPERTIES
            LINK_DEPENDS ${PROJECT_SOURCE_DIR}/capi/src/symbols-elf.map
        )
    elseif(CMAKE_C_COMPILER_ID MATCHES AppleClang)
        target_link_options(libchewing
            PRIVATE LINKER:-exported_symbols_list,${PROJECT_SOURCE_DIR}/capi/src/symbols-mach_o.map
            PRIVATE LINKER:-dead_strip
        )
        set_target_properties(libchewing PROPERTIES
            LINK_DEPENDS ${PROJECT_SOURCE_DIR}/capi/src/symbols-mach_o.map
        )
    elseif(MSVC)
        target_link_options(libchewing
            PRIVATE /DEF:${PROJECT_SOURCE_DIR}/capi/src/symbols-msvc.def
            PRIVATE /NODEFAULTLIB:MSVCRT
            PRIVATE /NODEFAULTLIB:MSVCRTD
            PRIVATE /NODEFAULTLIB:LIBCMTD
        )
        set_target_properties(libchewing PROPERTIES
            LINK_DEPENDS ${PROJECT_SOURCE_DIR}/capi/src/symbols-msvc.def
        )
    endif()
endif()

if(MSVC)
    set_target_properties(libchewing PROPERTIES
        OUTPUT_NAME chewing-msvc
    )
else()
    set_target_properties(libchewing PROPERTIES
        OUTPUT_NAME chewing
        SOVERSION 3
        VERSION 3.3.1
    )
endif()

install(FILES ${ALL_INC} DESTINATION ${CMAKE_INSTALL_INCLUDEDIR}/chewing)
install(FILES ${PROJECT_BINARY_DIR}/chewing.pc
    DESTINATION ${CMAKE_INSTALL_LIBDIR}/pkgconfig)
install(TARGETS libchewing
    EXPORT libchewingTargets
    ARCHIVE DESTINATION ${CMAKE_INSTALL_LIBDIR}
    INCLUDES DESTINATION ${CMAKE_INSTALL_INCLUDEDIR}
    LIBRARY DESTINATION ${CMAKE_INSTALL_LIBDIR}
    RUNTIME DESTINATION ${CMAKE_INSTALL_BINDIR})

install(IMPORTED_RUNTIME_ARTIFACTS chewing-cli DESTINATION ${CMAKE_INSTALL_BINDIR})

# generate CMake Config files
include(CMakePackageConfigHelpers)
set(CONFIG_PACKAGE_DIR "${CMAKE_INSTALL_LIBDIR}/cmake/Chewing")
configure_package_config_file(ChewingConfig.cmake.in
    "${CMAKE_CURRENT_BINARY_DIR}/ChewingConfig.cmake"
    INSTALL_DESTINATION ${CONFIG_PACKAGE_DIR})
write_basic_package_version_file("${CMAKE_CURRENT_BINARY_DIR}/ChewingConfigVersion.cmake"
    VERSION ${LIBCHEWING_VERSION}
    COMPATIBILITY SameMajorVersion)
export(EXPORT libchewingTargets
    NAMESPACE Chewing::
    FILE "${CMAKE_CURRENT_BINARY_DIR}/ChewingTargets.cmake")
install(FILES "${CMAKE_CURRENT_BINARY_DIR}/ChewingConfig.cmake"
              "${CMAKE_CURRENT_BINARY_DIR}/ChewingConfigVersion.cmake"
    DESTINATION ${CONFIG_PACKAGE_DIR})
install(EXPORT libchewingTargets
    FILE ChewingTargets.cmake
    NAMESPACE Chewing::
	DESTINATION ${CONFIG_PACKAGE_DIR})

set(CPACK_PACKAGE_CHECKSUM SHA256)
set(CPACK_PACKAGE_VERSION ${CMAKE_PROJECT_VERSION})
set(CPACK_SOURCE_IGNORE_FILES "/build" "/out" "/target" "/\\\\..*")
set(CPACK_SOURCE_GENERATOR TZST)
set(CPACK_SOURCE_PACKAGE_FILE_NAME libchewing-${CMAKE_PROJECT_VERSION})
include(CPack)
