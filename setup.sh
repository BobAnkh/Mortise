#!/bin/bash

echo 'Setup up'

# echo 'Checking if vagrant is installed with required version(2.3.1)...'
# VAGRANT_VERSION="$(vagrant -v)"
# if [[ "$VAGRANT_VERSION" == *"2.3.1" ]]; then
#     echo 'Exists vagrant 2.3.1...skip intstall!'
# else
#     echo 'No exists ...install vagrant 2.3.1!'
#     wget -c https://releases.hashicorp.com/vagrant/2.3.1/vagrant_2.3.1-1_amd64.deb
#     sudo dpkg -i vagrant_2.3.1-1_amd64.deb
#     sudo rm vagrant_2.3.1-1_amd64.deb
# fi

echo 'Installing rust'
echo "Rustc check..."
if command -v rustc >/dev/null 2>&1; then
	echo 'Exists rustc...skip intstall!'
else
	echo 'No exists rustc...install rust!'
	sudo apt install curl -y
	curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
	# shellcheck disable=SC1091
	source "$HOME"/.cargo/env
fi

echo 'Update apt dependencies'
sudo apt-get update -y
sudo apt install libelf-dev python-dev -y
# echo 'Installing nfs'
# sudo apt-get install nfs-common nfs-kernel-server -y

echo 'Installing vagrant and libvirt...'
curl -O https://raw.githubusercontent.com/vagrant-libvirt/vagrant-libvirt-qa/main/scripts/install.bash
chmod a+x ./install.bash
./install.bash || exit 1
rm ./install.bash

echo 'Installing vagrant plugins...'
vagrant plugin install vagrant-rsync-back

echo 'Enable the ports used by nfs...'
# 192.168.121.x
if command -v ufw >/dev/null 2>&1; then
	sudo ufw allow from 192.168.121.0/24 || echo "Fail to set ufw rule..."
else
	echo "Please do remember to allow the connections from the private network of VM in your firewall."
fi

echo 'Grant the user privilege... '
sudo usermod -aG kvm "$USER"
sudo usermod -aG libvirt "$USER"

echo 'Set up done.'
echo 'ATTENTION: Please exit and re-login your account to make the privilege take effect.'
echo 'Then run "vagrant up" and "vagrant ssh" to connect to the guest.'
echo 'You might face some issues while running vagrant up.'
echo 'Possibie issues and solutions are listed at README.md.'
