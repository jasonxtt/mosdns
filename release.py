# !/usr/bin/env python3
import argparse
import logging
import os
import subprocess
import zipfile

parser = argparse.ArgumentParser()
parser.add_argument("-i", type=int)
args = parser.parse_args()

PROJECT_NAME = 'mosdns'
RELEASE_DIR = './release'

logger = logging.getLogger(__name__)

# more info: https://golang.org/doc/install/source
# [(env : value),(env : value)]
envs = [
    [['GOOS', 'android'], ['GOARCH', 'arm64']],
    [['GOOS', 'darwin'], ['GOARCH', 'amd64']],
    [['GOOS', 'darwin'], ['GOARCH', 'arm64']],
    [['GOOS', 'windows'], ['GOARCH', 'amd64']],
    [['GOOS', 'windows'], ['GOARCH', 'amd64'], ['GOAMD64', 'v3']],
    [['GOOS', 'windows'], ['GOARCH', 'arm64']],
    [['GOOS', 'linux'], ['GOARCH', 'amd64']],
    [['GOOS', 'linux'], ['GOARCH', 'amd64'], ['GOAMD64', 'v3']],
    [['GOOS', 'linux'], ['GOARCH', 'arm64']],
    [['GOOS', 'linux'], ['GOARCH', 'arm'], ['GOARM', '5']],
    [['GOOS', 'linux'], ['GOARCH', 'arm'], ['GOARM', '6']],
    [['GOOS', 'linux'], ['GOARCH', 'arm'], ['GOARM', '7']],
    [['GOOS', 'linux'], ['GOARCH', 'mips']],
    [['GOOS', 'linux'], ['GOARCH', 'mips'], ['GOMIPS', 'softfloat']],
    [['GOOS', 'linux'], ['GOARCH', 'mips64']],
    [['GOOS', 'linux'], ['GOARCH', 'mips64'], ['GOMIPS64', 'softfloat']],
    [['GOOS', 'linux'], ['GOARCH', 'mipsle']],
    [['GOOS', 'linux'], ['GOARCH', 'mipsle'], ['GOMIPS', 'softfloat']],
    [['GOOS', 'linux'], ['GOARCH', 'mips64le']],
    [['GOOS', 'linux'], ['GOARCH', 'mips64le'], ['GOMIPS64', 'softfloat']]
]


def go_build():
    logger.info(f'building {PROJECT_NAME}')

    global envs
    if args.i is not None:
        envs = [envs[args.i]]

    # --- Compose version in the form: <base>-<yyyymmdd>-<shortsha>
    # Priority order for base: $VERSION (env) -> latest tag -> 'dev'
    base = os.getenv('VERSION')
    if not base:
        try:
            base = subprocess.check_output('git describe --tags --abbrev=0', shell=True).decode().strip()
        except subprocess.CalledProcessError:
            base = 'dev'

    # yyyymmdd in local time (can override via BUILD_DATE)
    build_date = os.getenv('BUILD_DATE')
    if not build_date:
        from time import strftime, localtime
        build_date = strftime('%Y%m%d', localtime())

    try:
        short_sha = subprocess.check_output('git rev-parse --short=7 HEAD', shell=True).decode().strip()
    except subprocess.CalledProcessError:
        short_sha = 'nogithash'

    VERSION = f"{base}-{build_date}-{short_sha}"

    try:
        subprocess.check_call('go run ../ config gen config.yaml', shell=True, env=os.environ)
    except Exception:
        logger.exception('failed to generate config template')
        raise

    for env in envs:
        os_env = os.environ.copy()  # new env

        s = PROJECT_NAME
        for pairs in env:
            os_env[pairs[0]] = pairs[1]  # add env
            s = s + '-' + pairs[1]
        zip_filename = s + '.zip'

        suffix = '.exe' if os_env['GOOS'] == 'windows' else ''
        bin_filename = PROJECT_NAME + suffix

        logger.info(f'building {zip_filename}')
        try:
            subprocess.check_call(
                f'go build -ldflags "-s -w -X main.version={VERSION}" -trimpath -o {bin_filename} ../', shell=True,
                env=os_env)

            with zipfile.ZipFile(zip_filename, mode='w', compression=zipfile.ZIP_DEFLATED,
                                 compresslevel=5) as zf:
                zf.write(bin_filename)
                zf.write('../README.md', 'README.md')
                zf.write('./config.yaml', 'config.yaml')
                zf.write('../LICENSE', 'LICENSE')

        except subprocess.CalledProcessError as e:
            logger.error(f'build {zip_filename} failed: {e.args}')
        except Exception:
            logger.exception('unknown err')


if __name__ == '__main__':
    logging.basicConfig(level=logging.INFO)

    if len(RELEASE_DIR) != 0:
        if not os.path.exists(RELEASE_DIR):
            os.mkdir(RELEASE_DIR)
        os.chdir(RELEASE_DIR)

    go_build()
