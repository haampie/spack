#!/bin/sh

# warmup
/usr/bin/python3 ~/spack/bin/spack list > /dev/null

/usr/bin/time -f '%E' /usr/bin/python3 ~/spack/bin/spack list > /dev/null
