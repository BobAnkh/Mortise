# -*- mode: ruby -*-
# vi: set ft=ruby :

Vagrant.configure("2") do |config|
    # use ubuntu 22.04 as the base box
    config.vm.box = "crystax/ubuntu2404"

    config.vm.synced_folder ".", "/vagrant", disabled: true
    # contain our CCA modules for debug purpose
    config.vm.synced_folder "./algorithm", "/home/vagrant/algorithm", type: 'rsync'
    config.vm.synced_folder "./mortise", "/home/vagrant/mortise", type: 'rsync'
    config.vm.synced_folder "./scripts", "/home/vagrant/scripts", type: 'rsync'
    # config.vm.synced_folder ".","/vagrant_data",type:'9p',accessmode:"squash",mount:"true"

    # ssh setting
    config.ssh.insert_key = true
    config.ssh.username = 'vagrant'
    config.ssh.password = 'vagrant'

    config.vm.network "private_network", ip: "192.168.121.5"
    config.vm.define "vm-kernel"

    #disk size:50GB 
    config.vm.disk :disk, size: "50GB", primary: true
    config.vm.provider:"libvirt" do |lv|
      # Customize the amount of memory on the VM:
      lv.machine_virtual_size = 128
      lv.memory = 16384
      lv.cpus = 6
      lv.cpu_mode = "host-passthrough"
    end

    config.vm.provision "shell", inline: "scripts/setup vm-new"
  end
