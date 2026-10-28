#!/bin/sh
set -eu

# /dev/kvm is root:kvm 0660 and the runner user is not in kvm.
echo 'KERNEL=="kvm", GROUP="kvm", MODE="0666", OPTIONS+="static_node=kvm"' |
	sudo tee /etc/udev/rules.d/99-kvm.rules >/dev/null
sudo udevadm control --reload-rules
sudo udevadm trigger --name-match=kvm
sudo udevadm settle
test -r /dev/kvm && test -w /dev/kvm

sudo apt-get update -qq
sudo apt-get install -y -qq build-essential e2fsprogs
